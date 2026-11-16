pub mod preview;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

pub const WIDTH: usize = 128;
pub const HEIGHT: usize = 32;
const GLYPH_WIDTH: usize = 5;
const GLYPH_HEIGHT: usize = 7;
const CHAR_ADVANCE: usize = 6;
const LINE_ADVANCE: usize = 8;
const SLIDE_MS: u64 = 800;
const MARQUEE_PAUSE_MS: u64 = 1000;
const MARQUEE_PIXELS_PER_SECOND: u64 = 12;

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
    pub page_interval_ms: u64,
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
            page_interval_ms: 5000,
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
                "page_interval_ms" => {
                    config.page_interval_ms = value
                        .parse()
                        .map_err(|_| format!("line {}: invalid page_interval_ms", index + 1))?;
                    if !(1000..=60000).contains(&config.page_interval_ms) {
                        return Err(format!(
                            "line {}: page_interval_ms must be between 1000 and 60000",
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

fn page_lines(lines: &[String], page_index: u64) -> Vec<&str> {
    if lines.is_empty() {
        return Vec::new();
    }
    let length = lines.len() + usize::from(lines.len() % 2 == 0);
    let start = ((u128::from(page_index / 2) * 5 + u128::from(page_index % 2) * 3) % length as u128)
        as usize;
    let count = if page_index % 2 == 0 { 3 } else { 2 };
    (0..count)
        .map(|offset| {
            lines
                .get((start + offset) % length)
                .map_or("", String::as_str)
        })
        .collect()
}

fn text_width(text: &str) -> usize {
    text.chars()
        .count()
        .saturating_mul(CHAR_ADVANCE)
        .saturating_sub(1)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Hold,
    Exit,
    Enter,
}

pub struct Animation {
    page_index: u64,
    phase: Phase,
    elapsed_ms: u64,
}

impl Default for Animation {
    fn default() -> Self {
        Self {
            page_index: 0,
            phase: Phase::Hold,
            elapsed_ms: 0,
        }
    }
}

impl Animation {
    fn hold_ms(&self, lines: &[String], minimum_ms: u64) -> u64 {
        let overflow = page_lines(lines, self.page_index)
            .iter()
            .map(|line| text_width(line).saturating_sub(WIDTH) as u64)
            .max()
            .unwrap_or(0);
        if overflow == 0 {
            return minimum_ms;
        }
        let travel_ms = overflow
            .saturating_mul(1000)
            .div_ceil(MARQUEE_PIXELS_PER_SECOND);
        minimum_ms.max(travel_ms.saturating_add(2 * MARQUEE_PAUSE_MS))
    }

    fn phase_duration_ms(&self, lines: &[String], minimum_ms: u64) -> u64 {
        match self.phase {
            Phase::Hold => self.hold_ms(lines, minimum_ms),
            Phase::Exit | Phase::Enter => SLIDE_MS,
        }
    }

    pub fn advance(&mut self, elapsed_ms: u64, lines: &[String], minimum_ms: u64) {
        self.elapsed_ms = self.elapsed_ms.saturating_add(elapsed_ms);
        loop {
            let duration = self.phase_duration_ms(lines, minimum_ms);
            if self.elapsed_ms < duration {
                break;
            }
            self.elapsed_ms -= duration;
            self.phase = match self.phase {
                Phase::Hold => Phase::Exit,
                Phase::Exit => {
                    self.page_index = self.page_index.wrapping_add(1);
                    Phase::Enter
                }
                Phase::Enter => Phase::Hold,
            };
        }
    }

    pub fn render(&self, lines: &[String], minimum_ms: u64) -> [u8; WIDTH * HEIGHT / 8] {
        let rows = page_lines(lines, self.page_index);
        let mut frame = [0; WIDTH * HEIGHT / 8];
        if rows.is_empty() {
            return frame;
        }
        let height = (rows.len() - 1) * LINE_ADVANCE + GLYPH_HEIGHT;
        let top = (HEIGHT - height) / 2;
        let marquee_ms = match self.phase {
            Phase::Enter => 0,
            Phase::Hold => self.elapsed_ms,
            Phase::Exit => self.hold_ms(lines, minimum_ms),
        };
        let marquee = marquee_ms
            .saturating_sub(MARQUEE_PAUSE_MS)
            .saturating_mul(MARQUEE_PIXELS_PER_SECOND)
            / 1000;
        for (row, text) in rows.iter().enumerate() {
            let width = text_width(text);
            let left = if width <= WIDTH {
                ((WIDTH - width) / 2) as isize
            } else {
                -(marquee.min((width - WIDTH) as u64) as isize)
            };
            draw_text(&mut frame, text, left, top + row * LINE_ADVANCE);
        }
        let distance = (self.elapsed_ms.min(SLIDE_MS) * WIDTH as u64 / SLIDE_MS) as isize;
        let offset = match self.phase {
            Phase::Hold => return frame,
            Phase::Exit => -distance,
            Phase::Enter => WIDTH as isize - distance,
        };
        let mut shifted = [0; WIDTH * HEIGHT / 8];
        for y in 0..HEIGHT {
            for x in 0..WIDTH {
                if frame[x + (y / 8) * WIDTH] & (1 << (y % 8)) != 0 {
                    set_pixel(&mut shifted, x as isize + offset, y);
                }
            }
        }
        shifted
    }
}

/// Number of pages before the 3/2 sequence returns to its initial position.
pub fn page_cycle_length(lines: &[String]) -> usize {
    if lines.is_empty() {
        return 0;
    }
    let length = lines.len() + usize::from(lines.len() % 2 == 0);
    if length % 5 == 0 {
        2 * (length / 5)
    } else {
        2 * length
    }
}

fn draw_text(frame: &mut [u8; WIDTH * HEIGHT / 8], text: &str, left: isize, top: usize) {
    for (character_index, character) in text.chars().enumerate() {
        let x = left + (character_index * CHAR_ADVANCE) as isize;
        if x >= WIDTH as isize {
            break;
        }
        if x + GLYPH_WIDTH as isize <= 0 {
            continue;
        }
        for (row, bits) in glyph(character.to_ascii_uppercase()).iter().enumerate() {
            for column in 0..GLYPH_WIDTH {
                if bits & (1 << (GLYPH_WIDTH - 1 - column)) != 0 {
                    set_pixel(frame, x + column as isize, top + row);
                }
            }
        }
    }
}

fn set_pixel(frame: &mut [u8; WIDTH * HEIGHT / 8], x: isize, y: usize) {
    if x < 0 || x >= WIDTH as isize || y >= HEIGHT {
        return;
    }
    let index = x as usize + (y / 8) * WIDTH;
    frame[index] |= 1 << (y % 8);
}

fn glyph(character: char) -> [u8; 7] {
    match character {
        'A' => [14, 17, 17, 31, 17, 17, 17],
        'B' => [30, 17, 17, 30, 17, 17, 30],
        'C' => [14, 17, 16, 16, 16, 17, 14],
        'D' => [30, 17, 17, 17, 17, 17, 30],
        'E' => [31, 16, 16, 30, 16, 16, 31],
        'F' => [31, 16, 16, 30, 16, 16, 16],
        'G' => [14, 17, 16, 23, 17, 17, 15],
        'H' => [17, 17, 17, 31, 17, 17, 17],
        'I' => [31, 4, 4, 4, 4, 4, 31],
        'J' => [7, 2, 2, 2, 18, 18, 12],
        'K' => [17, 18, 20, 24, 20, 18, 17],
        'L' => [16, 16, 16, 16, 16, 16, 31],
        'M' => [17, 27, 21, 21, 17, 17, 17],
        'N' => [17, 25, 21, 19, 17, 17, 17],
        'O' => [14, 17, 17, 17, 17, 17, 14],
        'P' => [30, 17, 17, 30, 16, 16, 16],
        'Q' => [14, 17, 17, 17, 21, 18, 13],
        'R' => [30, 17, 17, 30, 20, 18, 17],
        'S' => [15, 16, 16, 14, 1, 1, 30],
        'T' => [31, 4, 4, 4, 4, 4, 4],
        'U' => [17, 17, 17, 17, 17, 17, 14],
        'V' => [17, 17, 17, 17, 17, 10, 4],
        'W' => [17, 17, 17, 21, 21, 21, 10],
        'X' => [17, 17, 10, 4, 10, 17, 17],
        'Y' => [17, 17, 10, 4, 4, 4, 4],
        'Z' => [31, 1, 2, 4, 8, 16, 31],
        '0' => [14, 17, 19, 21, 25, 17, 14],
        '1' => [4, 12, 4, 4, 4, 4, 14],
        '2' => [14, 17, 1, 2, 4, 8, 31],
        '3' => [30, 1, 1, 14, 1, 1, 30],
        '4' => [2, 6, 10, 18, 31, 2, 2],
        '5' => [31, 16, 16, 30, 1, 1, 30],
        '6' => [14, 16, 16, 30, 17, 17, 14],
        '7' => [31, 1, 2, 4, 8, 8, 8],
        '8' => [14, 17, 17, 14, 17, 17, 14],
        '9' => [14, 17, 17, 15, 1, 1, 14],
        ':' => [0, 4, 4, 0, 4, 4, 0],
        '.' => [0, 0, 0, 0, 0, 4, 4],
        '/' => [1, 1, 2, 4, 8, 16, 16],
        '%' => [17, 2, 4, 8, 16, 17, 0],
        '-' => [0, 0, 0, 31, 0, 0, 0],
        '_' => [0, 0, 0, 0, 0, 0, 31],
        '+' => [0, 4, 4, 31, 4, 4, 0],
        ' ' => [0; 7],
        _ => [14, 17, 1, 2, 4, 0, 4],
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
    fn config_accepts_multiple_disks_and_page_settings() {
        let config = Config::parse(
            "show_ipv4=false\nnetwork_interface=eth0\ndisk=/\ndisk=/mnt/data\nscroll_speed=8.5\npage_interval_ms=7000\ni2c_address=0x3d\n",
        )
        .unwrap();

        assert!(!config.show_ipv4);
        assert_eq!(config.network_interface.as_deref(), Some("eth0"));
        assert_eq!(
            config.disks,
            [PathBuf::from("/"), PathBuf::from("/mnt/data")]
        );
        assert_eq!(config.scroll_speed, 8.5);
        assert_eq!(config.page_interval_ms, 7000);
        assert_eq!(config.i2c_address, 0x3d);
    }

    #[test]
    fn config_rejects_bad_values_and_relative_mounts() {
        assert!(Config::parse("scroll_speed=0\n").is_err());
        assert!(Config::parse("page_interval_ms=999\n").is_err());
        assert!(Config::parse("page_interval_ms=60001\n").is_err());
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
    fn alternating_pages_follow_the_requested_sequence() {
        let five: Vec<String> = (1..=5).map(|n| n.to_string()).collect();
        assert_eq!(page_lines(&five, 0), ["1", "2", "3"]);
        assert_eq!(page_lines(&five, 1), ["4", "5"]);
        assert_eq!(page_lines(&five, 2), ["1", "2", "3"]);
        assert_eq!(page_cycle_length(&five), 2);

        let six: Vec<String> = (1..=6).map(|n| n.to_string()).collect();
        let expected = [
            vec!["1", "2", "3"],
            vec!["4", "5"],
            vec!["6", "", "1"],
            vec!["2", "3"],
            vec!["4", "5", "6"],
            vec!["", "1"],
            vec!["2", "3", "4"],
        ];
        for (index, rows) in expected.iter().enumerate() {
            assert_eq!(&page_lines(&six, index as u64), rows);
        }
        assert_eq!(page_cycle_length(&six), 14);
        assert_eq!(page_lines(&six, 14), page_lines(&six, 0));
        assert!(page_lines(&[], 0).is_empty());
    }

    #[test]
    fn slides_exit_left_then_enter_right_and_center_rows() {
        let lines = vec!["H".to_owned(); 5];
        let mut animation = Animation::default();
        let centered = animation.render(&lines, 5000);
        assert_ne!(centered[61] & (1 << 4), 0);
        assert_ne!(centered[61 + WIDTH] & (1 << 4), 0);
        assert_ne!(centered[61 + 2 * WIDTH] & (1 << 4), 0);
        animation.advance(5000, &lines, 5000);
        animation.advance(400, &lines, 5000);
        let exiting = animation.render(&lines, 5000);
        assert_ne!(exiting[1] & (1 << 4), 0);
        animation.advance(400, &lines, 5000);
        assert_eq!(animation.render(&lines, 5000), [0; WIDTH * HEIGHT / 8]);
        animation.advance(400, &lines, 5000);
        let entering = animation.render(&lines, 5000);
        assert_ne!(entering[125 + WIDTH] & 1, 0);
        animation.advance(400, &lines, 5000);
        let two_rows = animation.render(&lines, 5000);
        assert_ne!(two_rows[61 + WIDTH] & 1, 0);
        assert_ne!(two_rows[61 + 2 * WIDTH] & 1, 0);
        assert_eq!(animation.render(&[], 5000), [0; WIDTH * HEIGHT / 8]);

        let mut skipped = Animation::default();
        skipped.advance(5000 + 2 * SLIDE_MS + 5000 + 400, &lines, 5000);
        assert_eq!(
            (skipped.page_index, skipped.phase, skipped.elapsed_ms),
            (1, Phase::Exit, 400)
        );
    }

    #[test]
    fn marquee_reaches_the_end_before_the_page_exits() {
        let lines = vec!["H".repeat(30); 5];
        let mut animation = Animation::default();
        let overflow = text_width(&lines[0]) - WIDTH;
        let duration = animation.hold_ms(&lines, 1000);
        assert!(
            duration
                >= MARQUEE_PAUSE_MS * 2
                    + (overflow as u64 * 1000).div_ceil(MARQUEE_PIXELS_PER_SECOND)
        );
        let first = animation.render(&lines, 1000);
        animation.advance(duration - 1, &lines, 1000);
        let last = animation.render(&lines, 1000);
        assert_ne!(first, last);
        let mut expected = [0; WIDTH * HEIGHT / 8];
        for row in 0..3 {
            draw_text(
                &mut expected,
                &lines[0],
                -(overflow as isize),
                4 + row * LINE_ADVANCE,
            );
        }
        assert_eq!(last, expected);
        assert_eq!(animation.phase, Phase::Hold);
        animation.advance(1, &lines, 1000);
        assert_eq!(animation.phase, Phase::Exit);
        assert_eq!(last, animation.render(&lines, 1000));
    }
}
