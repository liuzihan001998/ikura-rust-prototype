#!/usr/bin/env python3
"""Export translations from the user's local isf_sc.bin into a UTF-8 VM overlay.

Requires pycryptodome: python3 -m pip install pycryptodome
The binary dictionary belongs to the existing Chinese patch. Use --decoded
with the original decoded .ISF files to resolve dialogue context keys. This
tool reads both locally and does not modify or redistribute the patch files.
"""

import argparse
import hashlib
import json
import struct
from collections import Counter
from pathlib import Path

from Crypto.Cipher import AES


def read_dictionary(path):
    data = path.read_bytes()
    records = {}
    position = 0
    while position < len(data):
        if position + 20 > len(data):
            raise ValueError(f"truncated patch record at {position:#x}")
        length = struct.unpack_from("<I", data, position + 16)[0]
        end = position + 20 + length
        if length > 1_000_000 or end > len(data):
            raise ValueError(f"invalid patch record at {position:#x}")
        key = data[position:position + 16]
        encrypted = data[position + 20:end]
        # The patch can contain multiple localized values for one source key.
        # Its dictionary lookup returns the first sorted entry for that key.
        records.setdefault(key, encrypted)
        position = position + 20 + ((length + 3) & ~3)
    return records


def lookup(records, source):
    """The patch hashes UTF-8 source text and decrypts its value with AES-CFB128.

    Its SHA-256 implementation keeps digest words in little-endian memory
    order. The XOR of the two 16-byte halves is the dictionary key; the halves
    themselves are the CFB IV and AES key.
    """
    digest = hashlib.sha256(source.encode("utf-8")).digest()
    words = struct.unpack(">8I", digest)
    internal = struct.pack("<8I", *words)
    key = struct.pack("<4I", *(words[i] ^ words[i + 4] for i in range(4)))
    encrypted = records.get(key)
    if encrypted is None:
        return None
    decoded = AES.new(internal[16:], AES.MODE_CFB, iv=internal[:16], segment_size=128).decrypt(encrypted)
    return decoded.decode("utf-8")


def decoded_string(data):
    return data.decode("cp932")


def pm_lines_strict(payload):
    """Follow the VM's PM commands and return the visible literal lines."""
    cursor = 1
    lines = []
    while cursor < len(payload):
        command = payload[cursor]
        cursor += 1
        if command == 3:
            lines.clear()
        elif command == 1:
            if cursor >= len(payload):
                raise ValueError("truncated PM color command")
            cursor += {0: 4, 1: 3}[payload[cursor]]
        elif command == 4:
            cursor += 1
        elif command in (8, 0x11):
            cursor += 4
        elif command == 0x13:
            if cursor >= len(payload):
                raise ValueError("truncated PM voice command")
            if payload[cursor] == 4:
                cursor += 3
            else:
                cursor = payload.index(0, cursor) + 1
        elif command == 0xFF:
            end = payload.index(0, cursor)
            lines.append(decoded_string(payload[cursor:end]))
            cursor = end + 1
        elif command not in (0, 6):
            raise ValueError(f"unknown PM command {command:#x}")
        if cursor > len(payload):
            raise ValueError("PM command exceeds payload")
    return lines


def pm_lines(payload):
    try:
        return pm_lines_strict(payload)
    except (ValueError, KeyError):
        # Some script instructions contain PM commands unsupported by the
        # prototype VM. Keep their literal text available for later support.
        lines = []
        cursor = 1
        while True:
            marker = payload.find(b"\xff", cursor)
            if marker < 0:
                break
            end = payload.find(b"\0", marker + 1)
            if end < 0:
                break
            lines.append(decoded_string(payload[marker + 1:end]))
            cursor = end + 1
        return lines


def export(scripts, records, decoded_dir=None):
    pack = {"pages": {}, "choices": {}, "chapters": {}, "names": {}}
    counts = Counter()
    for script_name, script in scripts.items():
        original = (decoded_dir / f"{script_name}.ISF").read_bytes() if decoded_dir else None
        for instruction in script["instructions"]:
            opcode = instruction["opcode"]
            payload = bytes(instruction["payload"])
            key = f"{script_name}:0x{instruction['offset']:X}"
            if opcode in (0x2B, 0xD6):
                content = payload if opcode == 0x2B else payload[:1] + payload[2:]
                if opcode == 0xD6 and len(payload) > 1 and payload[1] == 0:
                    continue
                lines = list(pm_lines(content))
                if not lines:
                    continue
                translated = []
                matches = 0
                search = instruction["offset"]
                search_end = search + len(payload) + 16
                for line in lines:
                    replacement = None
                    if original is not None and line:
                        encoded = line.encode("cp932") + b"\0"
                        found = original.find(encoded, search, search_end)
                        if found >= 0:
                            search = found + len(encoded)
                            # The Windows hook hashes the UTF-8 text followed
                            # by // and 32 uppercase hex bytes, beginning at
                            # the string's NUL terminator in the ISF stream.
                            context = original[search - 1:search + 31]
                            if len(context) == 32:
                                replacement = lookup(records, line + "//" + context.hex().upper())
                                if replacement is not None:
                                    counts["dialogue_context"] += 1
                    if replacement is None:
                        replacement = lookup(records, line)
                    counts["dialogue_total"] += 1
                    if replacement is not None:
                        counts["dialogue_matched"] += 1
                        matches += 1
                    translated.append(replacement if replacement is not None else line)
                if matches:
                    pack["pages"][key] = {"speaker": None, "body": "\n".join(translated)}
            elif opcode == 0x15 and len(payload) > 18:
                source = decoded_string(payload[18:].split(b"\0", 1)[0])
                counts["choice_total"] += 1
                translated = lookup(records, source)
                if translated is not None:
                    counts["choice_matched"] += 1
                    pack["choices"][key] = translated
            elif opcode == 0xF7:
                source = decoded_string(payload.split(b"\0", 1)[0])
                counts["chapter_total"] += 1
                translated = lookup(records, source)
                if translated is not None:
                    counts["chapter_matched"] += 1
                    pack["chapters"][key] = translated
            elif opcode == 0x25 and len(payload) > 2:
                source = decoded_string(payload[2:].split(b"\0", 1)[0])
                counts["name_total"] += 1
                translated = lookup(records, source)
                if translated is not None:
                    counts["name_matched"] += 1
                    pack["names"][source] = translated
    return pack, counts


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--patch", type=Path, required=True, help="the local isf_sc.bin")
    parser.add_argument("--scripts", type=Path, required=True, help="original exported scripts.json")
    parser.add_argument("--decoded", type=Path, help="directory of original decoded .ISF files; enables context matches")
    parser.add_argument("--output", type=Path, required=True, help="UTF-8 translation pack")
    args = parser.parse_args()
    records = read_dictionary(args.patch)
    scripts = json.loads(args.scripts.read_text(encoding="utf-8"))
    pack, counts = export(scripts, records, args.decoded)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(pack, ensure_ascii=False, separators=(",", ":")), encoding="utf-8")
    print(f"{len(records)} unique patch records")
    for kind in ("dialogue", "choice", "chapter", "name"):
        print(f"{kind}: {counts[kind + '_matched']} / {counts[kind + '_total']} exact matches")
    if args.decoded:
        print(f"dialogue context matches: {counts['dialogue_context']}")
    print(f"wrote {args.output}")


if __name__ == "__main__":
    main()
