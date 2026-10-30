use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

pub const WIDTH: usize = 128;
pub const HEIGHT: usize = 32;
pub const LINE_SPACING: f32 = 17.0;

#[derive(Debug, Clone)]
pub struct Config {
    pub show_ipv4: bool,
    pub show_temperature: bool,
    pub show_disks: bool,
    pub network_interface: Option<String>,
    pub disks: Vec<PathBuf>,
    pub i2c_device: PathBuf,
    pub i2c_address: u8,
    pub scroll_speed: f32,
    pub refresh_ms: u64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            show_ipv4: true,
            show_temperature: true,
            show_disks: true,
            network_interface: None,
            disks: vec![PathBuf::from("/")],
            i2c_device: PathBuf::from("/dev/i2c-1"),
            i2c_address: 0x3c,
            scroll_speed: 5.0,
            refresh_ms: 100,
        }
    }
}

impl Config {
    pub fn from_file(path: &Path) -> Result<Self, String> {
        let contents = fs::read_to_string(path)
            .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
        Self::parse(&contents)
    }

    pub fn parse(contents: &str) -> Result<Self, String> {
        let mut config = Self::default();
        let mut has_disk = false;

        for (index, line) in contents.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let (key, value) = line
                .split_once('=')
                .ok_or_else(|| format!("line {}: expected key=value", index + 1))?;
            let key = key.trim();
            let value = value.trim();
            if value.is_empty() && key != "network_interface" {
                return Err(format!("line {}: {key} cannot be empty", index + 1));
            }

            match key {
                "show_ipv4" => config.show_ipv4 = parse_bool(key, value)?,
                "show_temperature" => config.show_temperature = parse_bool(key, value)?,
                "show_disks" => config.show_disks = parse_bool(key, value)?,
                "network_interface" => {
                    config.network_interface = (!value.is_empty()).then(|| value.to_owned());
                }
                "disk" => {
                    if !Path::new(value).is_absolute() {
                        return Err(format!("line {}: disk paths must be absolute", index + 1));
                    }
                    if !has_disk {
                        config.disks.clear();
                        has_disk = true;
                    }
                    config.disks.push(PathBuf::from(value));
                }
                "i2c_device" => config.i2c_device = PathBuf::from(value),
                "i2c_address" => {
                    let address = if let Some(hex) = value.strip_prefix("0x") {
                        u8::from_str_radix(hex, 16)
                    } else {
                        value.parse()
                    }
                    .map_err(|_| format!("line {}: invalid I2C address", index + 1))?;
                    if !(0x03..=0x77).contains(&address) {
                        return Err(format!(
                            "line {}: I2C address must be 0x03..=0x77",
                            index + 1
                        ));
                    }
                    config.i2c_address = address;
                }
                "scroll_speed" => {
                    config.scroll_speed = value
                        .parse()
                        .map_err(|_| format!("line {}: invalid scroll_speed", index + 1))?;
                    if !config.scroll_speed.is_finite()
                        || !(0.1..=100.0).contains(&config.scroll_speed)
                    {
                        return Err(format!(
                            "line {}: scroll_speed must be between 0.1 and 100 pixels/second",
                            index + 1
                        ));
                    }
                }
                "refresh_ms" => {
                    config.refresh_ms = value
                        .parse()
                        .map_err(|_| format!("line {}: invalid refresh_ms", index + 1))?;
                    if !(20..=2000).contains(&config.refresh_ms) {
                        return Err(format!(
                            "line {}: refresh_ms must be between 20 and 2000",
                            index + 1
                        ));
                    }
                }
                _ => return Err(format!("line {}: unknown setting {key}", index + 1)),
            }
        }

        Ok(config)
    }
}

fn parse_bool(key: &str, value: &str) -> Result<bool, String> {
    match value {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(format!("{key} must be true or false")),
    }
}

pub fn collect_lines(config: &Config) -> Vec<String> {
    let mut lines = Vec::new();
    if config.show_ipv4 {
        let addresses = ipv4_addresses(config.network_interface.as_deref());
        if addresses.is_empty() {
            lines.push("IP NO ADDRESS".to_owned());
        } else {
            lines.extend(addresses);
        }
    }
    if config.show_temperature {
        let temperatures = temperatures();
        if temperatures.is_empty() {
            lines.push("TEMP N/A".to_owned());
        } else {
            lines.extend(temperatures);
        }
    }
    if config.show_disks {
        lines.extend(config.disks.iter().map(|path| disk_line(path)));
    }
    lines
}

fn ipv4_addresses(interface: Option<&str>) -> Vec<String> {
    let output = Command::new("ip")
        .args(["-o", "-4", "addr", "show", "scope", "global"])
        .output();
    let Ok(output) = output else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    parse_ipv4_output(&String::from_utf8_lossy(&output.stdout), interface)
}

fn parse_ipv4_output(output: &str, interface: Option<&str>) -> Vec<String> {
    output
        .lines()
        .filter_map(|line| {
            let fields: Vec<_> = line.split_whitespace().collect();
            let name = fields.get(1)?.trim_end_matches(':').split('@').next()?;
            if interface.is_some_and(|wanted| wanted != name) {
                return None;
            }
            let address = fields
                .windows(2)
                .find(|fields| fields[0] == "inet")?
                .get(1)?
                .split('/')
                .next()?;
            Some(format!("IP {name} {address}"))
        })
        .collect()
}

fn temperatures() -> Vec<String> {
    let root = Path::new("/sys/class/thermal");
    let Ok(entries) = fs::read_dir(root) else {
        return Vec::new();
    };
    let mut zones: Vec<_> = entries.flatten().collect();
    zones.sort_by_key(|entry| entry.file_name());

    zones
        .into_iter()
        .filter_map(|entry| {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if !name.starts_with("thermal_zone") {
                return None;
            }
            let temperature = fs::read_to_string(entry.path().join("temp"))
                .ok()?
                .trim()
                .parse::<i64>()
                .ok()?;
            let kind = fs::read_to_string(entry.path().join("type")).unwrap_or_default();
            let kind = kind.trim().to_ascii_uppercase();
            let label = if kind.contains("CPU") || kind.contains("SOC") {
                "CPU".to_owned()
            } else if kind.is_empty() {
                name.to_ascii_uppercase()
            } else {
                kind.chars().take(7).collect()
            };
            Some(format!("{label} TEMP {}C", temperature / 1000))
        })
        .collect()
}

fn disk_line(path: &Path) -> String {
    let label = path
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or("ROOT");
    let output = Command::new("df")
        .args(["-B1", "--output=size,used,pcent"])
        .arg("--")
        .arg(path)
        .output();
    let Ok(output) = output else {
        return format!("DISK {label} N/A");
    };
    if !output.status.success() {
        return format!("DISK {label} N/A");
    }
    match parse_df_output(&String::from_utf8_lossy(&output.stdout)) {
        Some((total, used, percent)) => format!(
            "DISK {label} {}/{} {percent}",
            bytes_to_human(used),
            bytes_to_human(total)
        ),
        None => format!("DISK {label} N/A"),
    }
}

fn parse_df_output(output: &str) -> Option<(u64, u64, String)> {
    let fields: Vec<_> = output.lines().nth(1)?.split_whitespace().collect();
    Some((
        fields.first()?.parse().ok()?,
        fields.get(1)?.parse().ok()?,
        fields.get(2)?.to_string(),
    ))
}

fn bytes_to_human(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "K", "M", "G", "T"];
    let mut amount = bytes as f64;
    let mut unit = 0;
    while amount >= 1024.0 && unit < UNITS.len() - 1 {
        amount /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes}B")
    } else {
        format!("{amount:.1}{}", UNITS[unit])
    }
}

pub fn render_frame(lines: &[String], scroll_offset: f32) -> [u8; WIDTH * HEIGHT / 8] {
    let mut frame = [0; WIDTH * HEIGHT / 8];
    if lines.is_empty() {
        return frame;
    }
    let offset = normalize_scroll_offset(scroll_offset, lines.len());

    for world_index in -2..(lines.len() as isize + 2) {
        let y = HEIGHT as f32 + world_index as f32 * LINE_SPACING - offset;
        let scale = scroll_scale(y);
        let line = &lines[world_index.rem_euclid(lines.len() as isize) as usize];
        draw_text(&mut frame, line, y, scale);
    }

    frame
}

pub fn normalize_scroll_offset(offset: f32, line_count: usize) -> f32 {
    if line_count == 0 {
        0.0
    } else {
        offset.rem_euclid(LINE_SPACING * line_count as f32)
    }
}

fn draw_text(frame: &mut [u8; WIDTH * HEIGHT / 8], text: &str, top: f32, scale: f32) {
    let text = text.to_ascii_uppercase();
    let advance = 4.0 * scale;
    let width = (text.chars().count() as f32 * advance - scale).max(0.0);
    let left = ((WIDTH as f32 - width) / 2.0).floor();
    let top = top.floor();

    for (character_index, character) in text.chars().enumerate() {
        let glyph = glyph(character);
        let origin_x = left + character_index as f32 * advance;
        for (row, bits) in glyph.iter().enumerate() {
            for column in 0..3 {
                if bits & (1 << (2 - column)) == 0 {
                    continue;
                }
                let x_start = (origin_x + column as f32 * scale).floor() as isize;
                let x_end = (origin_x + (column + 1) as f32 * scale).ceil() as isize;
                let y_start = (top + row as f32 * scale).floor() as isize;
                let y_end = (top + (row + 1) as f32 * scale).ceil() as isize;
                for y in y_start..y_end {
                    for x in x_start..x_end {
                        set_pixel(frame, x, y);
                    }
                }
            }
        }
    }
}

fn set_pixel(frame: &mut [u8; WIDTH * HEIGHT / 8], x: isize, y: isize) {
    if x < 0 || y < 0 || x >= WIDTH as isize || y >= HEIGHT as isize {
        return;
    }
    let index = x as usize + (y as usize / 8) * WIDTH;
    frame[index] |= 1 << (y as usize % 8);
}

fn glyph(character: char) -> [u8; 5] {
    match character {
        'A' => [2, 5, 7, 5, 5],
        'B' => [6, 5, 6, 5, 6],
        'C' => [3, 4, 4, 4, 3],
        'D' => [6, 5, 5, 5, 6],
        'E' => [7, 4, 6, 4, 7],
        'F' => [7, 4, 6, 4, 4],
        'G' => [3, 4, 5, 5, 3],
        'H' => [5, 5, 7, 5, 5],
        'I' => [7, 2, 2, 2, 7],
        'J' => [1, 1, 1, 5, 2],
        'K' => [5, 5, 6, 5, 5],
        'L' => [4, 4, 4, 4, 7],
        'M' => [5, 7, 7, 5, 5],
        'N' => [5, 7, 7, 7, 5],
        'O' => [2, 5, 5, 5, 2],
        'P' => [6, 5, 6, 4, 4],
        'Q' => [2, 5, 5, 3, 1],
        'R' => [6, 5, 6, 5, 5],
        'S' => [3, 4, 2, 1, 6],
        'T' => [7, 2, 2, 2, 2],
        'U' => [5, 5, 5, 5, 7],
        'V' => [5, 5, 5, 5, 2],
        'W' => [5, 5, 7, 7, 5],
        'X' => [5, 5, 2, 5, 5],
        'Y' => [5, 5, 2, 2, 2],
        'Z' => [7, 1, 2, 4, 7],
        '0' => [7, 5, 5, 5, 7],
        '1' => [2, 6, 2, 2, 7],
        '2' => [6, 1, 2, 4, 7],
        '3' => [6, 1, 2, 1, 6],
        '4' => [5, 5, 7, 1, 1],
        '5' => [7, 4, 6, 1, 6],
        '6' => [3, 4, 6, 5, 2],
        '7' => [7, 1, 2, 2, 2],
        '8' => [2, 5, 2, 5, 2],
        '9' => [2, 5, 3, 1, 6],
        ':' => [0, 2, 0, 2, 0],
        '.' => [0, 0, 0, 0, 2],
        '/' => [1, 1, 2, 4, 4],
        '%' => [5, 1, 2, 4, 5],
        '-' => [0, 0, 7, 0, 0],
        '_' => [0, 0, 0, 0, 7],
        '+' => [0, 2, 7, 2, 0],
        ' ' => [0; 5],
        _ => [7, 1, 2, 0, 2],
    }
}

pub struct I2cDisplay {
    file: fs::File,
}

impl I2cDisplay {
    pub fn new(device: &Path, address: u8) -> io::Result<Self> {
        use std::os::fd::AsRawFd;
        use std::os::raw::{c_int, c_ulong};

        unsafe extern "C" {
            fn ioctl(fd: c_int, request: c_ulong, ...) -> c_int;
        }

        let file = fs::OpenOptions::new().read(true).write(true).open(device)?;
        const I2C_SLAVE: c_ulong = 0x0703;
        let result = unsafe { ioctl(file.as_raw_fd(), I2C_SLAVE, address as c_ulong) };
        if result < 0 {
            return Err(io::Error::last_os_error());
        }

        let mut display = Self { file };
        display.commands(&[
            0xae, 0xd5, 0x80, 0xa8, 0x1f, 0xd3, 0x00, 0x40, 0x8d, 0x14, 0x20, 0x02, 0xa1, 0xc8,
            0xda, 0x02, 0x81, 0x8f, 0xd9, 0xf1, 0xdb, 0x40, 0xa4, 0xa6, 0xaf,
        ])?;
        Ok(display)
    }

    pub fn show(&mut self, frame: &[u8; WIDTH * HEIGHT / 8]) -> io::Result<()> {
        for page in 0..(HEIGHT / 8) {
            self.commands(&[0xb0 + page as u8, 0x00, 0x10])?;
            let mut data = Vec::with_capacity(WIDTH + 1);
            data.push(0x40);
            data.extend_from_slice(&frame[page * WIDTH..(page + 1) * WIDTH]);
            self.file.write_all(&data)?;
        }
        Ok(())
    }

    fn commands(&mut self, commands: &[u8]) -> io::Result<()> {
        let mut data = Vec::with_capacity(commands.len() + 1);
        data.push(0x00);
        data.extend_from_slice(commands);
        self.file.write_all(&data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_accepts_multiple_disks_and_custom_scroll_settings() {
        let config = Config::parse(
            "show_ipv4=false\nnetwork_interface=eth0\ndisk=/\ndisk=/mnt/data\nscroll_speed=8.5\ni2c_address=0x3d\n",
        )
        .unwrap();

        assert!(!config.show_ipv4);
        assert_eq!(config.network_interface.as_deref(), Some("eth0"));
        assert_eq!(
            config.disks,
            [PathBuf::from("/"), PathBuf::from("/mnt/data")]
        );
        assert_eq!(config.scroll_speed, 8.5);
        assert_eq!(config.i2c_address, 0x3d);
    }

    #[test]
    fn config_rejects_bad_values_and_relative_mounts() {
        assert!(Config::parse("scroll_speed=0\n").is_err());
        assert!(Config::parse("disk=relative\n").is_err());
        assert!(Config::parse("i2c_address=0x80\n").is_err());
    }

    #[test]
    fn ipv4_parser_filters_interface_and_drops_prefix() {
        let output =
            "2: eth0: <UP> inet 192.168.1.20/24 brd 192.168.1.255\n3: wlan0: <UP> inet 10.0.0.4/24";
        assert_eq!(
            parse_ipv4_output(output, Some("eth0")),
            ["IP eth0 192.168.1.20"]
        );
    }

    #[test]
    fn disk_output_parser_and_byte_formatter_are_stable() {
        assert_eq!(
            parse_df_output("Size Used Use%\n1073741824 536870912 50%\n"),
            Some((1_073_741_824, 536_870_912, "50%".to_owned()))
        );
        assert_eq!(bytes_to_human(1024), "1.0K");
        assert_eq!(bytes_to_human(7), "7B");
    }

    #[test]
    fn lines_shrink_as_they_scroll_up_and_frames_change() {
        let lines = vec!["IP 192.168.1.2".to_owned(), "CPU TEMP 45C".to_owned()];
        assert!(render_frame(&lines, 0.0).iter().any(|byte| *byte != 0));
        assert_ne!(render_frame(&lines, 0.0), render_frame(&lines, 7.0));
        assert!(scroll_scale(32.0) > scroll_scale(-32.0));
        assert_eq!(normalize_scroll_offset(100.0, 2), 32.0);
    }
}

fn scroll_scale(y: f32) -> f32 {
    (3.0 - (32.0 - y).clamp(0.0, 64.0) / 32.0).clamp(1.0, 3.0)
}
