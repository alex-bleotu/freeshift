import argparse
import datetime
import io
import struct
import sys
import zipfile

ATT_CID = 0x0004
SMP_CID = 0x0006
H4_ACL = 0x02
H4_EVENT = 0x04
EPOCH_DELTA = 0x00DCDDB30F2F8000

OPCODES = {
    0x01: "error",
    0x02: "mtu request",
    0x03: "mtu response",
    0x08: "read by type request",
    0x09: "read by type response",
    0x0A: "read request",
    0x0B: "read response",
    0x10: "read by group request",
    0x11: "read by group response",
    0x12: "write request",
    0x13: "write response",
    0x1B: "notification",
    0x1D: "indication",
    0x1E: "confirmation",
    0x52: "write command",
}

SMP_CODES = {
    0x01: "pairing request",
    0x02: "pairing response",
    0x03: "pairing confirm",
    0x04: "pairing random",
    0x05: "pairing failed",
    0x0B: "security request",
    0x0C: "pairing public key",
    0x0D: "pairing DHKey check",
}


def uuid_text(raw):
    if len(raw) == 2:
        return f"{struct.unpack('<H', raw)[0]:04x}"
    if len(raw) == 16:
        value = raw[::-1].hex()
        return f"{value[0:8]}-{value[8:12]}-{value[12:16]}-{value[16:20]}-{value[20:32]}"
    return raw.hex()


def load(path):
    if path.lower().endswith(".zip"):
        with zipfile.ZipFile(path) as archive:
            candidates = [name for name in archive.namelist() if "btsnoop" in name.lower() and not name.endswith("/")]
            if not candidates:
                sys.exit(f"no btsnoop log inside {path}; was the HCI snoop log enabled before recording?")
            candidates.sort(key=lambda name: archive.getinfo(name).file_size, reverse=True)
            print(f"# using {candidates[0]} from {path}")
            return archive.read(candidates[0])
    with open(path, "rb") as handle:
        return handle.read()


def records(data):
    if data[:8] != b"btsnoop\x00":
        sys.exit("not a btsnoop file (missing header)")
    stream = io.BytesIO(data[16:])
    while True:
        header = stream.read(24)
        if len(header) < 24:
            return
        _, included, flags, _, timestamp = struct.unpack(">IIIIq", header)
        packet = stream.read(included)
        received = bool(flags & 0x01)
        seconds = (timestamp - EPOCH_DELTA) / 1_000_000
        try:
            moment = datetime.datetime.fromtimestamp(seconds).strftime("%H:%M:%S.%f")[:-3]
        except (OverflowError, OSError, ValueError):
            moment = "??:??:??"
        yield moment, received, packet


class Connection:
    def __init__(self):
        self.buffer = b""
        self.expected = 0
        self.handles = {}
        self.pending_read = None


def describe_att(connection, pdu):
    opcode = pdu[0]
    name = OPCODES.get(opcode, f"opcode 0x{opcode:02x}")
    body = pdu[1:]
    if opcode in (0x12, 0x52, 0x1B, 0x1D, 0x0A) and len(body) >= 2:
        handle = struct.unpack("<H", body[:2])[0]
        label = connection.handles.get(handle, "")
        value = body[2:]
        if opcode == 0x0A:
            connection.pending_read = handle
        return f"{name} handle 0x{handle:04x} {label} value {value.hex(' ')}".replace("  ", " ")
    if opcode == 0x0B:
        handle = connection.pending_read
        label = connection.handles.get(handle, "") if handle is not None else ""
        return f"{name} {label} value {body.hex(' ')}"
    if opcode == 0x09 and body:
        size = body[0]
        entries = body[1:]
        found = []
        for offset in range(0, len(entries) - size + 1, size):
            entry = entries[offset:offset + size]
            properties = entry[2]
            value_handle = struct.unpack("<H", entry[3:5])[0]
            uuid = uuid_text(entry[5:])
            connection.handles[value_handle] = f"[{uuid}]"
            found.append(f"0x{value_handle:04x}={uuid} props 0x{properties:02x}")
        return f"{name} characteristics " + ", ".join(found)
    if opcode == 0x11 and body:
        size = body[0]
        entries = body[1:]
        found = []
        for offset in range(0, len(entries) - size + 1, size):
            entry = entries[offset:offset + size]
            start, end = struct.unpack("<HH", entry[:4])
            found.append(f"0x{start:04x}-0x{end:04x}={uuid_text(entry[4:])}")
        return f"{name} services " + ", ".join(found)
    return f"{name} {body.hex(' ')}"


def main():
    parser = argparse.ArgumentParser(description="Print BLE ATT and pairing traffic from an Android btsnoop log or bugreport zip.")
    parser.add_argument("path", help="btsnoop_hci.log or the zip from adb bugreport")
    parser.add_argument("--all", action="store_true", help="also print discovery and MTU traffic")
    arguments = parser.parse_args()

    connections = {}
    names = {}
    count = 0
    for moment, received, packet in records(load(arguments.path)):
        if not packet:
            continue
        if packet[0] == H4_EVENT and len(packet) > 3 and packet[1] == 0x3E and packet[3] in (0x01, 0x0A):
            status = packet[4]
            handle = struct.unpack("<H", packet[5:7])[0] & 0x0FFF
            address = ":".join(f"{byte:02X}" for byte in packet[9:15][::-1])
            if status == 0:
                names[handle] = address
                print(f"{moment}  connection 0x{handle:03x} opened to {address}")
            continue
        if packet[0] != H4_ACL or len(packet) < 5:
            continue
        header, length = struct.unpack("<HH", packet[1:5])
        handle = header & 0x0FFF
        boundary = (header >> 12) & 0x03
        payload = packet[5:5 + length]
        connection = connections.setdefault(handle, Connection())
        if boundary in (0x00, 0x02):
            if len(payload) < 4:
                continue
            l2cap_length = struct.unpack("<H", payload[:2])[0]
            connection.buffer = payload
            connection.expected = l2cap_length + 4
        else:
            connection.buffer += payload
        if len(connection.buffer) < connection.expected:
            continue
        frame = connection.buffer[:connection.expected]
        connection.buffer = b""
        cid = struct.unpack("<H", frame[2:4])[0]
        pdu = frame[4:]
        if not pdu:
            continue
        direction = "<- device" if received else "-> device"
        peer = names.get(handle, f"0x{handle:03x}")
        if cid == SMP_CID:
            print(f"{moment}  {peer} {direction}  SMP {SMP_CODES.get(pdu[0], hex(pdu[0]))} {pdu[1:].hex(' ')}")
            count += 1
        elif cid == ATT_CID:
            text = describe_att(connection, pdu)
            is_noise = pdu[0] in (0x02, 0x03, 0x08, 0x10, 0x13, 0x1E, 0x01) or (pdu[0] in (0x09, 0x11) and not arguments.all)
            if is_noise and not arguments.all:
                continue
            print(f"{moment}  {peer} {direction}  {text}")
            count += 1
    print(f"# {count} lines")


if __name__ == "__main__":
    main()
