import { NapiCli } from "@napi-rs/cli";
import fs from "node:fs/promises";

const cli = new NapiCli();

const release = process.env["RELEASE"] === "1";

(async function build() {
  const target = process.env["TARGET"];
  if (release) {
    console.log(
      `Building node-addon in release mode for ${
        target || "current os and arch"
      }...`
    );
  } else {
    console.log(
      `Building node-addon in debug mode for ${
        target || "current os and arch"
      }...`
    );
  }
  const { task } = await cli.build({
    release,
    platform: true,
    esm: true,
    target,
  });
  await task;

  console.log("Adding QueryPlan definitions...");
  const queryPlanTypeDefs = await fs.readFile("src/query-plan.d.ts", "utf8");
  await fs.appendFile("index.d.ts", `\n${queryPlanTypeDefs}`);

  console.log("Ok");
})();
