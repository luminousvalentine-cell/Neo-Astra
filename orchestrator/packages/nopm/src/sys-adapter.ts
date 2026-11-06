import { spawnSync } from "node:child_process";

import { NopmError } from "./index.js";
import { defaultCommandRunner, type CommandRunner } from "./command.js";

export interface SysToolPlan {
  readonly tool: string;
  readonly manager: string | null;
  readonly commands: readonly (readonly string[])[];
  readonly installed: boolean;
}

export interface SysInstallContext {
  readonly platform: NodeJS.Platform;
  readonly uid: number;
  readonly managers: readonly string[];
}

const TOOLS: Record<string, { winget: string; brew: string; linux: string }> = {
  git: { winget: "Git.Git", brew: "git", linux: "git" },
  cmake: { winget: "Kitware.CMake", brew: "cmake", linux: "cmake" },
  clang: { winget: "LLVM.LLVM", brew: "llvm", linux: "clang" },
};

const KNOWN_TOOLS = Object.keys(TOOLS);

export function isSysToolId(id: string): boolean {
  return id.startsWith("sys:");
}

export function assertKnownSysTool(tool: string): void {
  if (!(tool in TOOLS)) {
    throw new NopmError(
      "NP0015",
      `Unknown system tool '${tool}'. Known tools: ${KNOWN_TOOLS.join(", ")}.`,
      tool,
    );
  }
}

export function detectManagers(
  platform: NodeJS.Platform = process.platform,
  presence: (command: string, args: readonly string[]) => boolean = (cmd, args) => {
    const r = spawnSync(cmd, [...args], { stdio: "ignore" });
    return r.status === 0;
  },
): string[] {
  const managers: string[] = [];

  if (platform === "win32") {
    if (presence("winget", ["--version"])) managers.push("winget");
    return managers;
  }
  if (platform === "darwin") {
    if (presence("brew", ["--version"])) managers.push("brew");
    return managers;
  }
  for (const m of ["apt-get", "dnf", "pacman", "zypper", "apk"]) {
    if (presence("sh", ["-c", `command -v ${m}`])) managers.push(m);
  }
  return managers;
}

export function planSysInstall(tool: string, context: SysInstallContext): SysToolPlan {
  assertKnownSysTool(tool);
  const spec = TOOLS[tool]!;

  if (context.platform === "win32") {
    if (!context.managers.includes("winget")) {
      throw new NopmError("NP0017", "No supported package manager found.", tool);
    }
    return {
      tool,
      manager: "winget",
      commands: [
        [
          "winget",
          "install",
          "--id",
          spec.winget,
          "-e",
          "--accept-package-agreements",
          "--accept-source-agreements",
          "--silent",
        ],
      ],
      installed: false,
    };
  }

  if (context.platform === "darwin") {
    if (context.uid === 0) {
      throw new NopmError("NP0017", "Homebrew refuses to run as root.", tool);
    }
    if (!context.managers.includes("brew")) {
      throw new NopmError("NP0017", "No supported package manager found.", tool);
    }
    return {
      tool,
      manager: "brew",
      commands: [["brew", "install", spec.brew]],
      installed: false,
    };
  }

  const linuxManagers = ["apt-get", "dnf", "pacman", "zypper", "apk"];
  const found = linuxManagers.find((m) => context.managers.includes(m));
  if (found === undefined) {
    throw new NopmError("NP0017", "No supported package manager found.", tool);
  }
  const pkg = spec.linux;
  const prefix = context.uid === 0 ? [] : ["sudo", "-n"];
  let commands: (readonly string[])[];
  switch (found) {
    case "apt-get":
      commands = [[...prefix, "apt-get", "update"], [...prefix, "apt-get", "install", "-y", pkg]];
      break;
    case "dnf":
      commands = [[...prefix, "dnf", "install", "-y", pkg]];
      break;
    case "pacman":
      commands = [[...prefix, "pacman", "-S", "--noconfirm", pkg]];
      break;
    case "zypper":
      commands = [[...prefix, "zypper", "--non-interactive", "install", pkg]];
      break;
    case "apk":
      commands = [[...prefix, "apk", "add", pkg]];
      break;
    default:
      throw new NopmError("NP0017", "No supported package manager found.", tool);
  }
  return {
    tool,
    manager: found,
    commands,
    installed: false,
  };
}

export function detectInstalled(tool: string, runner: CommandRunner): boolean {
  const result = runner.run(tool, ["--version"], {});
  return result.status === 0;
}

export function executeSysInstall(
  tool: string,
  context: SysInstallContext,
  runner: CommandRunner = defaultCommandRunner,
): SysToolPlan {
  assertKnownSysTool(tool);
  if (detectInstalled(tool, runner)) {
    return { tool, manager: null, commands: [], installed: true };
  }
  const plan = planSysInstall(tool, context);
  for (const cmd of plan.commands) {
    const [bin, ...args] = cmd;
    if (bin === undefined) continue;
    const result = runner.run(bin, args, { stdio: "inherit" });
    if (result.status !== 0) {
      const commandLine = [bin, ...args].join(" ");
      throw new NopmError(
        "NP0018",
        `System install failed: ${commandLine}\n${result.stderr}`.trim(),
        tool,
      );
    }
  }
  return { ...plan, installed: false };
}
