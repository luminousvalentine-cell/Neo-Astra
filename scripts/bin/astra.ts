import { basename } from "node:path";
import { runCli } from "../../orchestrator/packages/cli/src/index.ts";
import { runNopm } from "../../orchestrator/packages/nopm/src/cli.ts";

const argv = process.argv.slice(2);
const invoked = basename(process.argv[0] ?? "").toLowerCase();
const isNopm =
  invoked.startsWith("nopm") ||
  process.env["ASTRA_TOOL"] === "nopm" ||
  argv[0] === "nopm";
const nopmArgv = argv[0] === "nopm" ? argv.slice(1) : argv;
const task = isNopm ? runNopm(nopmArgv) : runCli(argv);

void task.then(
  (code) => {
    process.exitCode = code;
  },
  (error: unknown) => {
    process.stderr.write(
      `${error instanceof Error ? error.message : String(error)}\n`,
    );
    process.exitCode = 1;
  },
);
