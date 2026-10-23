#!/usr/bin/env node
// Turns a dhat-heap.json into a gzipped pprof profile, so `go tool pprof` can read it.
//
//     node bench/dhat-to-pprof.mjs dhat-heap.json heap.pb.gz
//
// Sample types (pick with -sample_index):
//   alloc_space / alloc_objects  everything allocated during the run
//   peak_space / peak_objects    what was live at the global heap peak
//   inuse_space / inuse_objects  what was still live at exit (leaks + long-lived state)
import { readFileSync, writeFileSync } from "node:fs";
import { gzipSync } from "node:zlib";

const SAMPLE_TYPES = [
  ["alloc_space", "bytes", "tb"],
  ["alloc_objects", "count", "tbk"],
  ["peak_space", "bytes", "gb"],
  ["peak_objects", "count", "gbk"],
  ["inuse_space", "bytes", "eb"],
  ["inuse_objects", "count", "ebk"],
];
// "0x1234: some::func (src/file.rs:12:5)", the address and location are optional
const FRAME_RE = /^(?:0x[0-9a-f]+: )?(.*?)(?: \((.+?):(\d+)(?::\d+)?\))?$/;
// allocator plumbing on top of every stack, dropped so the leaf is the code that asked for memory.
// inlined frames have no module path, so those are caught by their file
const ALLOC_FRAME_RE =
  /^(?:0x[0-9a-f]+: )?(?:backtrace::|<dhat::|dhat::|__rust_|__rdl_|<?alloc::(?:alloc|raw_vec)::|.*\((?:alloc\/)?src\/(?:alloc\.rs|raw_vec\/))/;

// plain arithmetic, not bit ops: those cut numbers to 32 bits and byte totals can be bigger
function varint(n) {
  const out = [];
  while (n >= 0x80) {
    out.push((n % 0x80) | 0x80);
    n = Math.floor(n / 0x80);
  }
  out.push(n);
  return Buffer.from(out);
}

const fieldVarint = (num, n) => Buffer.concat([varint(num * 8), varint(n)]);
const fieldBytes = (num, data) => Buffer.concat([varint(num * 8 + 2), varint(data.length), data]);
const packed = (num, values) => fieldBytes(num, Buffer.concat(values.map(varint)));

function main(src, dst) {
  const dhat = JSON.parse(readFileSync(src, "utf8"));
  const strings = new Map([["", 0]]);
  const s = (value) => {
    if (!strings.has(value)) strings.set(value, strings.size);
    return strings.get(value);
  };

  const functions = [];
  const locations = [];
  const frameToLocation = new Map();

  const locationId = (frameIdx) => {
    if (!frameToLocation.has(frameIdx)) {
      const [, name, file, line] = FRAME_RE.exec(dhat.ftbl[frameIdx]);
      const fid = functions.length + 1;
      functions.push(Buffer.concat([fieldVarint(1, fid), fieldVarint(2, s(name)), fieldVarint(4, s(file ?? ""))]));
      const lid = locations.length + 1;
      const lineMsg = Buffer.concat([fieldVarint(1, fid), fieldVarint(2, Number(line ?? 0))]);
      locations.push(Buffer.concat([fieldVarint(1, lid), fieldBytes(4, lineMsg)]));
      frameToLocation.set(frameIdx, lid);
    }
    return frameToLocation.get(frameIdx);
  };

  const out = [];
  for (const [name, unit] of SAMPLE_TYPES) {
    out.push(fieldBytes(1, Buffer.concat([fieldVarint(1, s(name)), fieldVarint(2, s(unit))])));
  }
  for (const pp of dhat.pps) {
    // dhat frames are leaf first, same as pprof; frame 0 is the synthetic "[root]"
    const frames = pp.fs.filter((f) => f !== 0);
    while (frames.length > 1 && ALLOC_FRAME_RE.test(dhat.ftbl[frames[0]])) frames.shift();
    const locIds = frames.map(locationId);
    const values = SAMPLE_TYPES.map(([, , key]) => pp[key] ?? 0);
    out.push(fieldBytes(2, Buffer.concat([packed(1, locIds), packed(2, values)])));
  }
  for (const loc of locations) out.push(fieldBytes(4, loc));
  for (const fn of functions) out.push(fieldBytes(5, fn));
  const defaultType = s("alloc_space");
  // a Map keeps insertion order, which is the id order
  for (const value of strings.keys()) out.push(fieldBytes(6, Buffer.from(value)));
  out.push(fieldVarint(14, defaultType));

  writeFileSync(dst, gzipSync(Buffer.concat(out)));
}

main(process.argv[2], process.argv[3]);
