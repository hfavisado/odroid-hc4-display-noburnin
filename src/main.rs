use odroid_hc4_display_noburnin::preview::{render_gif, sample_lines};
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
    let options = parse_arguments()?;
    let config_path = options.config;
    let config = match Config::from_file(&config_path) {
        Ok(config) => config,
        Err(_) if config_path.to_str() == Some(CONFIG_PATH) && !config_path.exists() => {
            Config::default()
        }
        Err(error) => return Err(error.into()),
    };
    if let Some(output) = options.preview {
        let lines = if options.live {
            collect_lines(&config)
        } else {
            sample_lines()
        };
        std::fs::write(
            &output,
            render_gif(&lines, config.scroll_speed, config.refresh_ms),
        )?;
        println!("wrote preview to {}", output.display());
        return Ok(());
    }
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

struct Options {
    config: PathBuf,
    preview: Option<PathBuf>,
    live: bool,
}

fn parse_arguments() -> Result<Options, String> {
    let mut options = Options {
        config: PathBuf::from(CONFIG_PATH),
        preview: None,
        live: false,
    };
    let mut arguments = std::env::args().skip(1);
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--config" => {
                options.config = arguments.next().ok_or("--config requires a path")?.into();
            }
            "--preview" => {
                options.preview = Some(arguments.next().ok_or("--preview requires a path")?.into());
            }
            "--live" => options.live = true,
            "--help" | "-h" => {
                println!(
                    "Usage: odroid-hc4-display [--config PATH] [--preview OUT.gif [--live]]\n\n\
                     --preview  write an animated GIF of the display instead of using the OLED\n\
                     --live     with --preview, use this machine's real data, not sample data"
                );
                std::process::exit(0);
            }
            other => return Err(format!("unknown argument {other}; use --help")),
        }
    }
    if options.live && options.preview.is_none() {
        return Err("--live requires --preview".to_owned());
    }
    Ok(options)
}
