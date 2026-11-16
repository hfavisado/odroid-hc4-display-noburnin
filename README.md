# odroid-hc4-display-noburnin

A small Rust program for the ODROID-HC4's 128×32 I²C OLED. It displays IPv4
addresses, thermal readings, and disk usage using a crisp 5×7 pixel font.
Pages alternate between three and two readings, shifting vertically between
visits to use the top and bottom margins. For an unchanged reading list, each
recurring page visits all vertical positions where its text fits equally often
over a complete cycle. Each page slides off to the left before the next slides
in from the right. Long readings scroll horizontally while the page is held.

![Animated preview of the paged OLED display](docs/preview.gif)

*Preview rendered with sample data; each lit pixel is one OLED pixel.*

## Build and run

The program has no Cargo dependencies and targets Linux. Build it on the HC4
(or cross-compile for its Linux architecture):

```sh
cargo build --release
sudo install -Dm755 target/release/odroid-hc4-display /usr/local/bin/odroid-hc4-display
sudo install -Dm644 odroid-hc4-display.conf /etc/odroid-hc4-display.conf
sudo /usr/local/bin/odroid-hc4-display
```

The example configuration is included in this repository. The built-in defaults
are used if `/etc/odroid-hc4-display.conf` is absent. Use `--config PATH` to
select another configuration. The process needs write access to the I²C device
(by default `/dev/i2c-1`); enable the Linux `i2c-dev` interface and run it as
root or grant the service account access to that device.

## Preview without hardware

`--preview` renders four to eight pages of the animation to a GIF and exits,
without touching the I²C device, so it works on any Linux/macOS machine with Rust:

```sh
cargo run --release -- --preview docs/preview.gif            # sample data
cargo run --release -- --preview out.gif --live              # this machine's IPs, temps, disks
cargo run --release -- --config my.conf --preview out.gif    # honours page_interval_ms
```

The GIF is written by a built-in encoder (no extra crates). It loops after a
short sample of the sequence; the real display continues through all vertical
positions. The one in this README is `docs/preview.gif`; regenerate it with the
first command after changing the rendering code.

## Configuration

The config file is plain `key=value`; blank lines and lines beginning with `#`
are ignored. Boolean values are `true` or `false`. For example:

```ini
show_ipv4=true
show_temperature=true
show_disks=true
network_interface=eth0
disk=/
disk=/mnt/data
i2c_device=/dev/i2c-1
i2c_address=0x3c
page_interval_ms=5000
refresh_ms=100
```

`network_interface` is optional; when unset, IPv4 addresses from all interfaces
with a global address are shown. Each `disk` line adds a mount path to display; the
default is `/`. Configure one line per mounted filesystem you want to monitor.
`page_interval_ms` is the minimum time a page remains stationary in milliseconds
(1000–60000). A page with a long reading stays longer if needed to show the
entire reading. `refresh_ms` is the display refresh interval in milliseconds
(20–2000); the sleep between animation frames is capped at 100 ms.
Existing `scroll_speed` settings are accepted for compatibility but no longer
affect the display. Disabling all three `show_*` settings leaves the display blank.
The list repeats continuously; if it has an even number of readings, one blank
line is appended before paging. Animation can spread usage but cannot guarantee
perfectly even wear.

## Data sources

- IPv4: `ip -o -4 addr show scope global` (`iproute2`).
- Temperature: Linux thermal-zone `type` and `temp` files under
  `/sys/class/thermal/`; on HC4 this includes the SoC thermal reading, reported
  by the kernel in thousandths of a degree Celsius.
- Disk usage: `df -B1 --output=size,used,pcent MOUNT_PATH` for each configured
  mount path (GNU coreutils).
- Display: SSD1306-compatible I²C device, configured with the I²C device path
  and 7-bit address.

Unavailable readings are skipped where possible; a missing IPv4 address,
temperature sensor, or mount is shown as `N/A`/`NO ADDRESS` rather than stopping
the display.

## Credits

Hardware and thermal-source information follows the
[ODROID-HC4 OLED documentation](https://wiki.odroid.com/odroid-hc4/application_note/oled).
The system-status display concept and storage configuration were informed by
[rpardini/sys-oled-hc4](https://github.com/rpardini/sys-oled-hc4), which in turn
credits the original [kobol-io/sys-oled](https://github.com/kobol-io/sys-oled).
