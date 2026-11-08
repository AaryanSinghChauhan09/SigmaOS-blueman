use sigmaos_blueman::BluetoothManager;

fn main() {
    if let Err(error) = run() {
        eprintln!("sigmaos-blueman: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let action = args.next().unwrap_or_else(|| "help".into());
    let manager = BluetoothManager::new("bluetoothctl");
    match action.as_str() {
        "status" => {
            let adapter = manager.adapter_info()?;
            println!(
                "Adapter {} ({})",
                adapter.name.as_deref().unwrap_or("unnamed"),
                adapter.address
            );
            println!("Powered: {}", adapter.powered);
            println!("Discoverable: {}", adapter.discoverable);
            println!("Pairable: {}", adapter.pairable);
        }
        "devices" => {
            let devices = manager.known_devices()?;
            if devices.is_empty() {
                println!("No known Bluetooth devices.");
            }
            for device in devices {
                println!("{}  {}", device.address, device.name);
            }
        }
        "pair" | "connect" | "disconnect" => {
            let address = args.next().ok_or("device address is required")?;
            if args.next().is_some() {
                return Err("unexpected extra argument".into());
            }
            match action.as_str() {
                "pair" => manager.pair(&address)?,
                "connect" => manager.connect(&address)?,
                _ => manager.disconnect(&address)?,
            }
        }
        "help" | "--help" | "-h" => {
            println!("Usage: sigmaos-blueman <status|devices|pair|connect|disconnect> [BLUETOOTH_ADDRESS]");
            println!("Pairing and connection commands call BlueZ directly and may require authorization.");
        }
        _ => return Err(format!("unknown command: {action}").into()),
    }
    Ok(())
}
