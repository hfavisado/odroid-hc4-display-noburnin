use odroid_hc4_display_noburnin::{
    Config, I2cDisplay, collect_lines, normalize_scroll_offset, render_frame,
};
use std::path::PathBuf;
use std::thread;
use std::time::{Duration, Instant};

const CONFIG_PATH: &str = "/etc/odroid-hc4-display.conf";
const DATA_REFRESH: Duration = Duration::from_secs(1);

fn main() {
    if let Err(error) = run() {
        eprintln!("odroid-hc4-display: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let config_path = config_argument()?;
    let config = match Config::from_file(&config_path) {
        Ok(config) => config,
        Err(_) if config_path.to_str() == Some(CONFIG_PATH) && !config_path.exists() => {
            Config::default()
        }
        Err(error) => return Err(error.into()),
    };
    let mut display = I2cDisplay::new(&config.i2c_device, config.i2c_address)?;
    let mut last_frame = Instant::now();
    let mut last_data_refresh = Instant::now();
    let mut scroll_offset = 0.0;
    let mut lines = collect_lines(&config);

    loop {
        display.show(&render_frame(&lines, scroll_offset))?;
        thread::sleep(Duration::from_millis(config.refresh_ms));
        let now = Instant::now();
        scroll_offset += config.scroll_speed * now.duration_since(last_frame).as_secs_f32();
        scroll_offset = normalize_scroll_offset(scroll_offset, lines.len());
        last_frame = now;
        if now.duration_since(last_data_refresh) >= DATA_REFRESH {
            lines = collect_lines(&config);
            scroll_offset = normalize_scroll_offset(scroll_offset, lines.len());
            last_data_refresh = now;
        }
    }
}

fn config_argument() -> Result<PathBuf, String> {
    let mut arguments = std::env::args().skip(1);
    match arguments.next().as_deref() {
        None => Ok(PathBuf::from(CONFIG_PATH)),
        Some("--config") => {
            let path = arguments.next().ok_or("--config requires a path")?;
            if arguments.next().is_some() {
                return Err("unexpected extra argument".to_owned());
            }
            Ok(PathBuf::from(path))
        }
        Some("--help" | "-h") => {
            println!("Usage: odroid-hc4-display [--config PATH]");
            std::process::exit(0);
        }
        Some(argument) => Err(format!("unknown argument {argument}; use --help")),
    }
}
