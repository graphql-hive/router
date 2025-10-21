import fs from "node:fs/promises";
import { NapiCli } from "@napi-rs/cli";

const cli = new NapiCli();

const release = process.env["RELEASE"] === "1";

(async function build() {
  if (release) {
    console.log("Building node-addon in release mode...");
  } else {
    console.log("Building node-addon in debug mode...");
  }
  const { task } = await cli.build({
    release,
    platform: true,
    esm: true,
  });
  await task;

  console.log("Adding QueryPlan definitions...");
  const queryPlanTypeDefs = await fs.readFile("src/query-plan.d.ts", "utf8");
  await fs.appendFile("index.d.ts", `\n${queryPlanTypeDefs}`);

  console.log("Ok");
})();
