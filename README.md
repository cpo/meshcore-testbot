# MeshCoreBot

MeshCoreBot is a companion bot for the MeshCore system, communicating over **USB serial** or **TCP**. It uses the same framing protocol as the MeshCore `ArduinoSerialInterface`:
- **Send:** `<` + u16 LE length + payload
- **Receive:** `>` + u16 LE length + payload

## Features
- Connects to MeshCore via USB serial or TCP (configurable by environment variables)
- Periodically polls for messages and logs all companion packets (optional)
- Replies to trigger messages (e.g., `Test` or `name: Test`) on the same channel index
- Customizable reply location text

## Usage
## Environment Variables

The following environment variables control the behavior of MeshCoreBot:

- `MESHCORE_SERIAL`: Path to the USB serial device (e.g., `/dev/ttyACM0`). Used for serial communication unless `MESHCORE_TCP` is set.
- `MESHCORE_BAUD`: Baud rate for serial communication (default: `115200`).
- `MESHCORE_TCP`: Host and port for TCP communication (e.g., `192.168.1.5:5000`). If set, TCP is used and `MESHCORE_SERIAL` is ignored.
- `MESHCORE_POLL_SECS`: Interval in seconds for periodic `GET_MESSAGE` polling (default: `3`).
- `MESHCORE_LOGALL`: If set (to any value), logs every companion packet sent and received (hex dump and parsed summary to stderr).
- `MESHCORE_REPLY_TEXT`: Customizes the location line in replies (default: `Den Bosch Noord`).

### Example

```sh
export MESHCORE_SERIAL=/dev/ttyACM0
export MESHCORE_BAUD=115200
export MESHCORE_POLL_SECS=5
export MESHCORE_LOGALL=1
export MESHCORE_REPLY_TEXT="Test Location"
cargo run --release
```
- **USB:** Set `MESHCORE_SERIAL` to the device path (e.g., `/dev/ttyACM0`). Optionally set `MESHCORE_BAUD` (default: `115200`).
- **TCP:** Set `MESHCORE_TCP` to `host:port` (e.g., `192.168.1.5:5000`). If set, TCP is used and `MESHCORE_SERIAL` is ignored.
- **Polling:** Set `MESHCORE_POLL_SECS` (default: `3`) for periodic polling.
- **Logging:** Set `MESHCORE_LOGALL` (any value) to log all packets in and out.
- **Reply Location:** Set `MESHCORE_REPLY_TEXT` to customize the location line in replies (default: `Den Bosch Noord`).

## Building

This project uses [Cargo](https://doc.rust-lang.org/cargo/):

```
cargo build --release
```

The resulting binary will be in `target/release/meshcorebot`.

## License

This project is licensed under the GNU Lesser General Public License (LGPL) v3.0 or later.

```
This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Lesser General Public License as published by
the Free Software Foundation, either version 3 of the License, or
(at your option) any later version.

This program is distributed in the hope that it will be useful,
but WITHOUT ANY WARRANTY; without even the implied warranty of
MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
GNU Lesser General Public License for more details.

You should have received a copy of the GNU Lesser General Public License
along with this program.  If not, see <https://www.gnu.org/licenses/>.
```
