import { exec } from "child_process";
import { readFileSync, writeFileSync } from "fs";
import path from "path";
import { fileURLToPath } from "url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));

console.log("generating typescript types...");

const gen_command =
  "RUST_BACKTRACE=1 typeshare . --lang=typescript --output-file=./client/core/ts/src/types.ts";

exec(gen_command, (error, _stdout, _stderr) => {
  if (error) {
    console.error(error);
    process.exitCode = 1;
    return;
  }
  console.log("generated types using typeshare");
  fix_types();
  fix_domain_responses();
  console.log("finished.");
});

function fix_types() {
  const types_path = __dirname + "/src/types.ts";
  const contents = readFileSync(types_path);
  const fixed = contents
    .toString()
    // Replace Variants
    .replaceAll("ResourceTargetVariant", 'ResourceTarget["type"]')
    .replaceAll("AlerterEndpointVariant", 'AlerterEndpoint["type"]')
    .replaceAll("AlertDataVariant", 'AlertData["type"]')
    .replaceAll("ServerTemplateConfigVariant", 'ServerTemplateConfig["type"]')
    .replaceAll("UserConfigVariant", 'UserConfig["type"]')
    // Add '| string' to env vars
    .replaceAll("EnvironmentVar[]", "EnvironmentVar[] | string")
    .replaceAll("IndexSet", "Array")
    .replaceAll(
      ": PermissionLevelAndSpecifics",
      ": PermissionLevelAndSpecifics | PermissionLevel"
    )
    .replaceAll(
      ", PermissionLevelAndSpecifics",
      ", PermissionLevelAndSpecifics | PermissionLevel"
    )
    .replaceAll("IndexMap", "Record")
    .replace(/[ \t]+$/gm, "");
  writeFileSync(types_path, fixed);
}

// Keep the new workflow's response map derived from the Rust Resolve contract.
function fix_domain_responses() {
  const responsesPath = path.join(__dirname, "src/responses.ts");
  let contents = readFileSync(responsesPath, "utf8");
  for (const kind of ["read", "write", "execute"]) {
    const source = readFileSync(path.join(__dirname, `../rs/src/api/${kind}/domain.rs`), "utf8");
    const entries = [...source.matchAll(/#\[response\(([^)]+)\)\][\s\S]*?pub struct (\w+)/g)].map(([, response, name]) => {
      const vector = response.match(/^Vec<(.+)>$/);
      const type = vector ? `Types.${vector[1]}[]` : `Types.${response}`;
      return `  ${name}: ${type};`;
    });
    if (!entries.length) throw new Error(`No domain ${kind} response contracts found`);
    const title = kind[0].toUpperCase() + kind.slice(1);
    const marker = `  // BEGIN GENERATED DOMAIN ${kind.toUpperCase()}\n`;
    const end = `  // END GENERATED DOMAIN ${kind.toUpperCase()}\n`;
    const startAt = contents.indexOf(marker);
    if (startAt >= 0) {
      const endAt = contents.indexOf(end, startAt);
      if (endAt < 0) throw new Error("Unclosed domain response generation marker");
      contents = contents.slice(0, startAt) + contents.slice(endAt + end.length);
    }
    contents = contents.replace(`export type ${title}Responses = {\n`, `export type ${title}Responses = {\n${marker}${entries.join("\n")}\n${end}`);
  }
  writeFileSync(responsesPath, contents);
}
