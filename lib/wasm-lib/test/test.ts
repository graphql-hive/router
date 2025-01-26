const pkg = await import("../pkg-node/wasm_lib.js");

pkg.init("type Query { test: String }");

export {};
