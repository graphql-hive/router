import { NapiCli } from "@napi-rs/cli";

const cli = new NapiCli();

(async function build() {
  console.log("waiting build");
  const { task } = await cli.build({
    release: process.env["RELEASE"] === "1",
    platform: true,
    esm: true,
    dts: "./index.d.ts",
    jsBinding: "./index.js",
  });
  console.log("waiting task");
  await task;
  console.log("ok");
})();
