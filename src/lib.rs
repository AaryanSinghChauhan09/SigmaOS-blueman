//! Bluetooth adapter and known-device management through BlueZ.
#![forbid(unsafe_code)]

use std::ffi::{OsStr, OsString};
use std::fmt;
use std::io;
use std::process::Command;

/// Current adapter state parsed from `bluetoothctl show`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdapterInfo {
    pub address: String,
    pub name: Option<String>,
    pub powered: bool,
    pub discoverable: bool,
    pub pairable: bool,
}

/// A known remote device reported by BlueZ.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BluetoothDevice {
    pub address: String,
    pub name: String,
}

/// Reject malformed Bluetooth device addresses before invoking BlueZ.
pub fn is_valid_address(address: &str) -> bool {
    let mut octets = address.split(':');
    let valid_octet =
        |octet: &str| octet.len() == 2 && octet.bytes().all(|byte| byte.is_ascii_hexdigit());
    (0..6).all(|_| octets.next().map(valid_octet).unwrap_or(false)) && octets.next().is_none()
}

/// Parse adapter state from `bluetoothctl show` output.
pub fn parse_adapter_info(output: &str) -> Result<AdapterInfo, BluetoothError> {
    let mut address = None;
    let mut name = None;
    let mut powered = None;
    let mut discoverable = None;
    let mut pairable = None;

    for line in output.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("Controller ") {
            let candidate = rest.split_whitespace().next().unwrap_or_default();
            if is_valid_address(candidate) {
                address = Some(candidate.to_owned());
            }
        } else if let Some(value) = line.strip_prefix("Name:") {
            name = Some(value.trim().to_owned());
        } else if let Some(value) = line.strip_prefix("Powered:") {
            powered = Some(parse_boolean(value)?);
        } else if let Some(value) = line.strip_prefix("Discoverable:") {
            discoverable = Some(parse_boolean(value)?);
        } else if let Some(value) = line.strip_prefix("Pairable:") {
            pairable = Some(parse_boolean(value)?);
        }
    }

    Ok(AdapterInfo {
        address: address
            .ok_or_else(|| BluetoothError::InvalidOutput("adapter address is missing".into()))?,
        name,
        powered: powered
            .ok_or_else(|| BluetoothError::InvalidOutput("powered state is missing".into()))?,
        discoverable: discoverable
            .ok_or_else(|| BluetoothError::InvalidOutput("discoverable state is missing".into()))?,
        pairable: pairable
            .ok_or_else(|| BluetoothError::InvalidOutput("pairable state is missing".into()))?,
    })
}

fn parse_boolean(value: &str) -> Result<bool, BluetoothError> {
    match value.trim() {
        "yes" => Ok(true),
        "no" => Ok(false),
        other => Err(BluetoothError::InvalidOutput(format!(
            "expected yes or no, got {other:?}"
        ))),
    }
}

/// Parse known-device lines of the form `Device AA:BB:CC:DD:EE:FF Device name`.
pub fn parse_devices(output: &str) -> Result<Vec<BluetoothDevice>, BluetoothError> {
    let mut devices = Vec::new();
    for (index, line) in output.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let invalid = || {
            BluetoothError::InvalidOutput(format!("invalid device line {}: {line:?}", index + 1))
        };
        let rest = line.strip_prefix("Device ").ok_or_else(invalid)?;
        let (address, name) = rest.split_once(' ').ok_or_else(invalid)?;
        if !is_valid_address(address) || name.trim().is_empty() {
            return Err(invalid());
        }
        devices.push(BluetoothDevice {
            address: address.to_ascii_uppercase(),
            name: name.trim().to_owned(),
        });
    }
    Ok(devices)
}

/// Process result returned by an injectable BlueZ command runner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandOutput {
    pub success: bool,
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

/// Command boundary for direct, shell-free BlueZ invocation.
pub trait CommandRunner {
    fn run(&self, program: &OsStr, args: &[&str], interactive: bool) -> io::Result<CommandOutput>;
}

/// Production runner for `bluetoothctl`.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemCommandRunner;

impl CommandRunner for SystemCommandRunner {
    fn run(&self, program: &OsStr, args: &[&str], interactive: bool) -> io::Result<CommandOutput> {
        let mut command = Command::new(program);
        command.args(args);
        if interactive {
            let status = command.status()?;
            Ok(CommandOutput {
                success: status.success(),
                exit_code: status.code(),
                stdout: String::new(),
                stderr: String::new(),
            })
        } else {
            let output = command.output()?;
            Ok(CommandOutput {
                success: output.status.success(),
                exit_code: output.status.code(),
                stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
                stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            })
        }
    }
}

/// Bluetooth manager failures.
#[derive(Debug)]
pub enum BluetoothError {
    Io(io::Error),
    CommandFailed {
        exit_code: Option<i32>,
        stderr: String,
    },
    InvalidAddress(String),
    InvalidOutput(String),
}

impl fmt::Display for BluetoothError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "could not run bluetoothctl: {error}"),
            Self::CommandFailed { exit_code, stderr } => write!(
                f,
                "bluetoothctl failed (exit {exit_code:?}): {}",
                stderr.trim()
            ),
            Self::InvalidAddress(address) => write!(f, "invalid Bluetooth address: {address}"),
            Self::InvalidOutput(detail) => write!(f, "invalid bluetoothctl output: {detail}"),
        }
    }
}

impl std::error::Error for BluetoothError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

/// BlueZ-backed adapter and known-device operations.
#[derive(Debug)]
pub struct BluetoothManager<R = SystemCommandRunner> {
    program: OsString,
    runner: R,
}

impl BluetoothManager<SystemCommandRunner> {
    /// Use `bluetoothctl` from `PATH`.
    pub fn new(program: impl Into<OsString>) -> Self {
        Self {
            program: program.into(),
            runner: SystemCommandRunner,
        }
    }
}

impl<R: CommandRunner> BluetoothManager<R> {
    /// Construct a manager with a custom runner for tests or alternate integrations.
    pub fn with_runner(program: impl Into<OsString>, runner: R) -> Self {
        Self {
            program: program.into(),
            runner,
        }
    }

    /// Read current adapter state.
    pub fn adapter_info(&self) -> Result<AdapterInfo, BluetoothError> {
        let output = self.run_checked(&["show"], false)?;
        parse_adapter_info(&output.stdout)
    }

    /// List devices already known to the BlueZ daemon.
    pub fn known_devices(&self) -> Result<Vec<BluetoothDevice>, BluetoothError> {
        let output = self.run_checked(&["devices"], false)?;
        parse_devices(&output.stdout)
    }

    /// Pair with a device. BlueZ receives the terminal for agent prompts.
    pub fn pair(&self, address: &str) -> Result<(), BluetoothError> {
        self.device_action("pair", address, true)
    }

    /// Connect to a paired or trusted device.
    pub fn connect(&self, address: &str) -> Result<(), BluetoothError> {
        self.device_action("connect", address, true)
    }

    /// Disconnect from a connected device.
    pub fn disconnect(&self, address: &str) -> Result<(), BluetoothError> {
        self.device_action("disconnect", address, true)
    }

    fn device_action(
        &self,
        action: &str,
        address: &str,
        interactive: bool,
    ) -> Result<(), BluetoothError> {
        if !is_valid_address(address) {
            return Err(BluetoothError::InvalidAddress(address.to_owned()));
        }
        self.run_checked(&[action, &address.to_ascii_uppercase()], interactive)
            .map(|_| ())
    }

    fn run_checked(
        &self,
        args: &[&str],
        interactive: bool,
    ) -> Result<CommandOutput, BluetoothError> {
        let output = self
            .runner
            .run(&self.program, args, interactive)
            .map_err(BluetoothError::Io)?;
        if output.success {
            Ok(output)
        } else {
            Err(BluetoothError::CommandFailed {
                exit_code: output.exit_code,
                stderr: output.stderr,
            })
        }
    }
}

/// Backwards-compatible component handle that owns the default BlueZ manager.
#[derive(Debug)]
pub struct Component {
    manager: BluetoothManager,
}

impl Component {
    /// Construct a component using `bluetoothctl` from `PATH`.
    pub fn new() -> Self {
        Self {
            manager: BluetoothManager::new("bluetoothctl"),
        }
    }

    /// Access the component's BlueZ manager.
    pub fn manager(&self) -> &BluetoothManager {
        &self.manager
    }
}

impl Default for Component {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    struct FakeRunner {
        output: CommandOutput,
        calls: RefCell<Vec<(OsString, Vec<String>, bool)>>,
    }

    impl FakeRunner {
        fn returning(stdout: &str) -> Self {
            Self {
                output: CommandOutput {
                    success: true,
                    exit_code: Some(0),
                    stdout: stdout.into(),
                    stderr: String::new(),
                },
                calls: RefCell::new(Vec::new()),
            }
        }
    }

    impl CommandRunner for FakeRunner {
        fn run(
            &self,
            program: &OsStr,
            args: &[&str],
            interactive: bool,
        ) -> io::Result<CommandOutput> {
            self.calls.borrow_mut().push((
                program.to_owned(),
                args.iter().map(|arg| (*arg).to_owned()).collect(),
                interactive,
            ));
            Ok(self.output.clone())
        }
    }

    #[test]
    fn parses_adapter_state() {
        let info = parse_adapter_info("Controller 01:23:45:67:89:AB (public)\n\tName: SigmaOS\n\tPowered: yes\n\tDiscoverable: no\n\tPairable: yes\n").unwrap();
        assert_eq!(info.address, "01:23:45:67:89:AB");
        assert_eq!(info.name.as_deref(), Some("SigmaOS"));
        assert!(info.powered);
        assert!(!info.discoverable);
        assert!(info.pairable);
    }

    #[test]
    fn parses_device_names_with_spaces() {
        let devices = parse_devices("Device aa:bb:cc:dd:ee:ff Office Headphones\n").unwrap();
        assert_eq!(
            devices,
            vec![BluetoothDevice {
                address: "AA:BB:CC:DD:EE:FF".into(),
                name: "Office Headphones".into()
            }]
        );
    }

    #[test]
    fn rejects_invalid_device_address_before_running_command() {
        let runner = FakeRunner::returning("");
        let manager = BluetoothManager::with_runner("bluetoothctl", runner);

        let result = manager.connect("not-an-address");

        assert!(matches!(result, Err(BluetoothError::InvalidAddress(_))));
        assert!(manager.runner.calls.borrow().is_empty());
    }

    #[test]
    fn connect_uses_direct_interactive_bluez_arguments() {
        let runner = FakeRunner::returning("");
        let manager = BluetoothManager::with_runner("bluetoothctl", runner);

        manager.connect("aa:bb:cc:dd:ee:ff").unwrap();

        let calls = manager.runner.calls.borrow();
        assert_eq!(calls[0].1, vec!["connect", "AA:BB:CC:DD:EE:FF"]);
        assert!(calls[0].2);
    }
}
