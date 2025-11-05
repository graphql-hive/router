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
    // we dont rebuild the bindings because they will regenerate the node addon npm package versions
    // and we dont use the npm packages for addons, they're all in this single npm package.
    // so we disable js binding generatation to not have to complicate knope unnecessarely
    // with workflow steps that update those versions in the binding.
    noJsBinding: true,
  });
  await task;

  console.log("Adding QueryPlan definitions...");
  const queryPlanTypeDefs = await fs.readFile("src/query-plan.d.ts", "utf8");
  await fs.appendFile("index.d.ts", `\n${queryPlanTypeDefs}`);

  console.log("Ok");
})();
