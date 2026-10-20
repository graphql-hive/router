#!/usr/bin/env python3
"""Turns a dhat-heap.json into a gzipped pprof profile, so `go tool pprof` can read it.

    python3 bench/dhat-to-pprof.py dhat-heap.json heap.pb.gz

Sample types (pick with -sample_index):
  alloc_space / alloc_objects  everything allocated during the run
  peak_space / peak_objects    what was live at the global heap peak
  inuse_space / inuse_objects  what was still live at exit (leaks + long-lived state)
"""
import gzip
import json
import re
import sys

SAMPLE_TYPES = [
    ("alloc_space", "bytes", "tb"),
    ("alloc_objects", "count", "tbk"),
    ("peak_space", "bytes", "gb"),
    ("peak_objects", "count", "gbk"),
    ("inuse_space", "bytes", "eb"),
    ("inuse_objects", "count", "ebk"),
]
# "0x1234: some::func (src/file.rs:12:5)", the address and location are optional
FRAME_RE = re.compile(r"^(?:0x[0-9a-f]+: )?(.*?)(?: \((.+?):(\d+)(?::\d+)?\))?$")
# allocator plumbing on top of every stack, dropped so the leaf is the code that asked for memory
ALLOC_FRAME_RE = re.compile(r"^(?:0x[0-9a-f]+: )?(?:<dhat::|dhat::|__rust_|__rdl_|<?alloc::(?:alloc|raw_vec)::|alloc \(alloc/src/alloc\.rs)")


def varint(n):
    out = bytearray()
    while True:
        b = n & 0x7F
        n >>= 7
        if n:
            out.append(b | 0x80)
        else:
            out.append(b)
            return bytes(out)


def field_varint(num, n):
    return varint(num << 3) + varint(n)


def field_bytes(num, data):
    return varint(num << 3 | 2) + varint(len(data)) + data


def packed(num, values):
    return field_bytes(num, b"".join(varint(v) for v in values))


def main(src, dst):
    dhat = json.load(open(src))
    strings = {"": 0}

    def s(value):
        return strings.setdefault(value, len(strings))

    functions, locations = [], []
    frame_to_location = {}

    def location_id(frame_idx):
        if frame_idx not in frame_to_location:
            name, file, line = FRAME_RE.match(dhat["ftbl"][frame_idx]).groups()
            fid = len(functions) + 1
            functions.append(field_varint(1, fid) + field_varint(2, s(name)) + field_varint(4, s(file or "")))
            lid = len(locations) + 1
            line_msg = field_varint(1, fid) + field_varint(2, int(line or 0))
            locations.append(field_varint(1, lid) + field_bytes(4, line_msg))
            frame_to_location[frame_idx] = lid
        return frame_to_location[frame_idx]

    out = bytearray()
    for name, unit, _ in SAMPLE_TYPES:
        out += field_bytes(1, field_varint(1, s(name)) + field_varint(2, s(unit)))
    for pp in dhat["pps"]:
        # dhat frames are leaf first, same as pprof; frame 0 is the synthetic "[root]"
        frames = [f for f in pp["fs"] if f != 0]
        while len(frames) > 1 and ALLOC_FRAME_RE.match(dhat["ftbl"][frames[0]]):
            frames.pop(0)
        loc_ids = [location_id(f) for f in frames]
        values = [pp.get(key, 0) for _, _, key in SAMPLE_TYPES]
        out += field_bytes(2, packed(1, loc_ids) + packed(2, values))
    for loc in locations:
        out += field_bytes(4, loc)
    for fn in functions:
        out += field_bytes(5, fn)
    default_type = s("alloc_space")
    for value in sorted(strings, key=strings.get):
        out += field_bytes(6, value.encode())
    out += field_varint(14, default_type)

    with gzip.open(dst, "wb") as f:
        f.write(out)


if __name__ == "__main__":
    main(*sys.argv[1:3])
