import { NapiCli } from "@napi-rs/cli";
import fs from "node:fs/promises";

const cli = new NapiCli();

const release = process.env["RELEASE"] === "true";
const crossCompile = process.env["CROSS_COMPILE"] === "true";

(async function build() {
  const target = process.env["TARGET"];
  console.log(
    `Building node-addon in ${release ? "release" : "debug"} mode for ${
      target || "current os and arch"
    }${crossCompile ? " with cross compile" : ""}...`
  );
  const { task } = await cli.build({
    release,
    platform: true,
    esm: true,
    crossCompile,
    target,
  });
  await task;

  console.log("Adding QueryPlan definitions...");
  const queryPlanTypeDefs = await fs.readFile("src/query-plan.d.ts", "utf8");
  await fs.appendFile("index.d.ts", `\n${queryPlanTypeDefs}`);

  console.log("Ok");
})();
