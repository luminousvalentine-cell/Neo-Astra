#![allow(dead_code)]

pub struct CatalogEntry {
    pub code: &'static str,
    pub edition: &'static str,
    pub meaning: &'static str,
}

pub const CATALOG: &[CatalogEntry] = &[
    CatalogEntry {
        code: "NP0001",
        edition: "0.1",
        meaning: "Invalid source-qualified package ID",
    },
    CatalogEntry {
        code: "NP0002",
        edition: "0.1",
        meaning: "Invalid dependency declaration or duplicate descriptor",
    },
    CatalogEntry {
        code: "NP0003",
        edition: "0.1",
        meaning: "Missing package descriptor",
    },
    CatalogEntry {
        code: "NP0004",
        edition: "0.1",
        meaning: "Dependency cycle",
    },
    CatalogEntry {
        code: "NP0005",
        edition: "0.1",
        meaning: "Conflicting root dependency alias",
    },
    CatalogEntry {
        code: "NP0006",
        edition: "0.1",
        meaning: "Package is incompatible with the selected target",
    },
    CatalogEntry {
        code: "NP0007",
        edition: "0.1",
        meaning: "Package capability is incompatible with the selected mode",
    },
    CatalogEntry {
        code: "NP0008",
        edition: "0.1",
        meaning: "Invalid lockfile, missing integrity, or unpinned Git dependency",
    },
    CatalogEntry {
        code: "NP0009",
        edition: "0.1",
        meaning: "Missing or stale lockfile",
    },
    CatalogEntry {
        code: "NP0010",
        edition: "0.1",
        meaning: "Lockfile write failure",
    },
    CatalogEntry {
        code: "NP0011",
        edition: "0.1",
        meaning: "Package adapter, path, fetch, or installation failure",
    },
    CatalogEntry {
        code: "NP0012",
        edition: "0.1",
        meaning: "NoPM command-line usage error",
    },
    CatalogEntry {
        code: "NP0013",
        edition: "0.1",
        meaning: "Dependency is already declared",
    },
    CatalogEntry {
        code: "NP0014",
        edition: "0.1",
        meaning: "Dependency is not declared",
    },
    CatalogEntry {
        code: "NP0015",
        edition: "0.1",
        meaning: "Unknown system tool",
    },
    CatalogEntry {
        code: "NP0016",
        edition: "0.1",
        meaning: "NoPM does not uninstall system tools",
    },
    CatalogEntry {
        code: "NP0017",
        edition: "0.1",
        meaning: "No supported package manager was found",
    },
    CatalogEntry {
        code: "NP0018",
        edition: "0.1",
        meaning: "System tool installation failed",
    },
    CatalogEntry {
        code: "NR0001",
        edition: "0.1",
        meaning: "Package already exists (HTTP 409)",
    },
    CatalogEntry {
        code: "NR0002",
        edition: "0.1",
        meaning: "Unknown registry route, or signature/public-key verification failed",
    },
    CatalogEntry {
        code: "NR0003",
        edition: "0.1",
        meaning: "Package or version not found",
    },
    CatalogEntry {
        code: "NR0004",
        edition: "0.1",
        meaning: "Package is yanked",
    },
    CatalogEntry {
        code: "NS1001",
        edition: "0.1",
        meaning: "Invalid comment form",
    },
    CatalogEntry {
        code: "NS1002",
        edition: "0.1",
        meaning: "Invalid escape sequence or byte-literal value",
    },
    CatalogEntry {
        code: "NS1003",
        edition: "0.1",
        meaning: "Unterminated string or byte literal",
    },
    CatalogEntry {
        code: "NS1004",
        edition: "0.1",
        meaning: "Reserved: invalid UTF-8 source is reported by the CLI as `NS9007`",
    },
    CatalogEntry {
        code: "NS1005",
        edition: "0.1",
        meaning: "Invalid identifier, malformed literal, or unexpected character",
    },
    CatalogEntry {
        code: "NS2001",
        edition: "0.1",
        meaning: "Expected token or delimiter",
    },
    CatalogEntry {
        code: "NS2002",
        edition: "0.1",
        meaning: "Invalid syntax",
    },
    CatalogEntry {
        code: "NS2003",
        edition: "0.1",
        meaning: "Invalid type",
    },
    CatalogEntry {
        code: "NS2004",
        edition: "0.1",
        meaning: "Invalid pattern",
    },
    CatalogEntry {
        code: "NS2005",
        edition: "0.1",
        meaning: "Invalid or unexpected item",
    },
    CatalogEntry {
        code: "NS2006",
        edition: "0.1",
        meaning: "Missing newline or `;` boundary",
    },
    CatalogEntry {
        code: "NS2007",
        edition: "0.1",
        meaning: "Invalid modifier",
    },
    CatalogEntry {
        code: "NS2008",
        edition: "0.2",
        meaning: "Unsupported or invalid edition",
    },
    CatalogEntry {
        code: "NS2009",
        edition: "0.2",
        meaning: "Deprecated canonical alias",
    },
    CatalogEntry {
        code: "NS2010",
        edition: "0.2",
        meaning: "Expression-position type arguments require `::<...>`",
    },
    CatalogEntry {
        code: "NS2011",
        edition: "0.2",
        meaning: "Malformed turbofish",
    },
    CatalogEntry {
        code: "NS2012",
        edition: "0.2",
        meaning: "Deprecated `spawn",
    },
    CatalogEntry {
        code: "NS2013",
        edition: "0.2",
        meaning: "Deprecated `/%`",
    },
    CatalogEntry {
        code: "NS2014",
        edition: "0.2",
        meaning: "Invalid tuple index",
    },
    CatalogEntry {
        code: "NS2015",
        edition: "0.2",
        meaning: "Deprecated or removed v0.1 spelling",
    },
    CatalogEntry {
        code: "NS2016",
        edition: "0.2",
        meaning: "Canonical-syntax lint family",
    },
    CatalogEntry {
        code: "NS3001",
        edition: "0.1",
        meaning: "Private declaration cannot be exported",
    },
    CatalogEntry {
        code: "NS3002",
        edition: "0.1",
        meaning: "Conflicting visibility modifiers",
    },
    CatalogEntry {
        code: "NS3003",
        edition: "0.1",
        meaning: "Duplicate export name",
    },
    CatalogEntry {
        code: "NS3004",
        edition: "0.1",
        meaning: "Duplicate default export",
    },
    CatalogEntry {
        code: "NS3005",
        edition: "0.1",
        meaning: "Unknown export target",
    },
    CatalogEntry {
        code: "NS3006",
        edition: "0.1",
        meaning: "Duplicate import binding",
    },
    CatalogEntry {
        code: "NS3007",
        edition: "0.1",
        meaning: "Imported name is not exported",
    },
    CatalogEntry {
        code: "NS3008",
        edition: "0.1",
        meaning: "Conflicting re-exports",
    },
    CatalogEntry {
        code: "NS3009",
        edition: "0.1",
        meaning: "Default export target is not a value",
    },
    CatalogEntry {
        code: "NS3010",
        edition: "0.1",
        meaning: "Private declaration leaks through a public API",
    },
    CatalogEntry {
        code: "NS3011",
        edition: "0.1",
        meaning: "Duplicate top-level declaration",
    },
    CatalogEntry {
        code: "NS3012",
        edition: "0.1",
        meaning: "Imported module cannot be resolved",
    },
    CatalogEntry {
        code: "NS3013",
        edition: "0.1",
        meaning: "Cyclic named import or re-export",
    },
    CatalogEntry {
        code: "NS4001",
        edition: "0.1",
        meaning: "Type mismatch",
    },
    CatalogEntry {
        code: "NS4002",
        edition: "0.1",
        meaning: "Unknown identifier",
    },
    CatalogEntry {
        code: "NS4003",
        edition: "0.1",
        meaning: "Unknown type",
    },
    CatalogEntry {
        code: "NS4004",
        edition: "0.1",
        meaning: "Wrong argument count",
    },
    CatalogEntry {
        code: "NS4005",
        edition: "0.1",
        meaning: "Value is not callable",
    },
    CatalogEntry {
        code: "NS4006",
        edition: "0.1",
        meaning: "Invalid operand types",
    },
    CatalogEntry {
        code: "NS4007",
        edition: "0.1",
        meaning: "Missing return on a reachable path",
    },
    CatalogEntry {
        code: "NS4008",
        edition: "0.1",
        meaning: "Reserved",
    },
    CatalogEntry {
        code: "NS4009",
        edition: "0.1",
        meaning: "Assignment to an immutable binding",
    },
    CatalogEntry {
        code: "NS4010",
        edition: "0.2",
        meaning: "Non-exhaustive match (including missing `Ok`/`Err`/error member)",
    },
    CatalogEntry {
        code: "NS4011",
        edition: "0.1",
        meaning: "Unhandled error effect",
    },
    CatalogEntry {
        code: "NS4012",
        edition: "0.2",
        meaning: "Error is not declared by the function",
    },
    CatalogEntry {
        code: "NS4013",
        edition: "0.2",
        meaning: "Invalid error propagation",
    },
    CatalogEntry {
        code: "NS4014",
        edition: "0.2",
        meaning: "Invalid control flow (`return fail`, and existing cases)",
    },
    CatalogEntry {
        code: "NS4015",
        edition: "0.1",
        meaning: "Invalid member or index access",
    },
    CatalogEntry {
        code: "NS4016",
        edition: "0.1",
        meaning: "Invalid, missing, or duplicate struct field",
    },
    CatalogEntry {
        code: "NS4017",
        edition: "0.2",
        meaning: "Invalid pattern, arity, or error-member pattern",
    },
    CatalogEntry {
        code: "NS4018",
        edition: "0.2",
        meaning: "Invalid type argument or arity (including wildcard arguments)",
    },
    CatalogEntry {
        code: "NS4019",
        edition: "0.1",
        meaning: "A value carrying a `Task` handle would cross a task boundary by copy: a channel element, a spawn capture, or a spawn result",
    },
    CatalogEntry {
        code: "NS5001",
        edition: "0.1",
        meaning: "Mode requirement is not satisfied",
    },
    CatalogEntry {
        code: "NS5002",
        edition: "0.1",
        meaning: "Mode widening is not allowed",
    },
    CatalogEntry {
        code: "NS5003",
        edition: "0.1",
        meaning: "Capability requirement is not satisfied",
    },
    CatalogEntry {
        code: "NS5004",
        edition: "0.1",
        meaning: "Manual memory requires system mode",
    },
    CatalogEntry {
        code: "NS5005",
        edition: "0.1",
        meaning: "Invalid capability or mode annotation",
    },
    CatalogEntry {
        code: "NS5006",
        edition: "0.1",
        meaning: "No single built-in mode satisfies a project requirement",
    },
    CatalogEntry {
        code: "NS5007",
        edition: "0.1",
        meaning: "System mode: two or more state-changing calls in a target-native-order position (binary operands, array elements, struct fields, map entries) - hoist the calls into bindings",
    },
    CatalogEntry {
        code: "NS6001",
        edition: "0.1",
        meaning: "Manual allocation outside a manual-memory scope",
    },
    CatalogEntry {
        code: "NS6002",
        edition: "0.1",
        meaning: "Manual allocation of a GC-managed element type",
    },
    CatalogEntry {
        code: "NS7001",
        edition: "0.1",
        meaning: "Unknown or unsupported script language",
    },
    CatalogEntry {
        code: "NS7002",
        edition: "0.1",
        meaning: "Script language is not supported by the active target",
    },
    CatalogEntry {
        code: "NS7003",
        edition: "0.1",
        meaning: "Missing, empty, or duplicate `@exports` declaration",
    },
    CatalogEntry {
        code: "NS7004",
        edition: "0.1",
        meaning: "Malformed script block: missing `@end` or a required header field",
    },
    CatalogEntry {
        code: "NS7005",
        edition: "0.1",
        meaning: "Invalid or non-boundary-safe script export signature",
    },
    CatalogEntry {
        code: "NS7006",
        edition: "0.1",
        meaning: "Script namespace collision",
    },
    CatalogEntry {
        code: "NS7007",
        edition: "0.1",
        meaning: "Missing or inconsistent manifest `ffi` mapping for a foreign module or a required inline-script link/runtime entry",
    },
    CatalogEntry {
        code: "NS7008",
        edition: "0.1",
        meaning: "Script backend or link failure, missing implementation export, or no pinned TypeScript transpiler",
    },
    CatalogEntry {
        code: "NS7010",
        edition: "0.1",
        meaning: "`@export_as` on a non-generic or non-exported function",
    },
    CatalogEntry {
        code: "NS7011",
        edition: "0.1",
        meaning: "`@export_as` binding list does not bind every type parameter exactly once",
    },
    CatalogEntry {
        code: "NS7012",
        edition: "0.1",
        meaning: "`@export_as` bound type is not C-boundary-safe",
    },
    CatalogEntry {
        code: "NS7013",
        edition: "0.1",
        meaning: "`@export_as` symbol is not a valid C identifier, or duplicates another symbol or instantiation",
    },
    CatalogEntry {
        code: "NS8001",
        edition: "0.1",
        meaning: "A backend or target does not support the requested construct or configuration",
    },
    CatalogEntry {
        code: "NS8002",
        edition: "0.1",
        meaning: "A backend request or response is malformed or incomplete: invalid typed module, missing module, output path, entry module, or expected artifact",
    },
    CatalogEntry {
        code: "NS8003",
        edition: "0.1",
        meaning: "Manual memory is not supported by the JavaScript or TypeScript backend",
    },
    CatalogEntry {
        code: "NS8004",
        edition: "0.1",
        meaning: "A generated artifact path escapes the build output directory",
    },
    CatalogEntry {
        code: "NS8017",
        edition: "0.1",
        meaning: "Calling this callee is not supported by the C backend first slice",
    },
    CatalogEntry {
        code: "NS8018",
        edition: "0.1",
        meaning: "A generic function requires monomorphization",
    },
    CatalogEntry {
        code: "NS8019",
        edition: "0.1",
        meaning: "Imports are not supported by the C backend first slice",
    },
    CatalogEntry {
        code: "NS8020",
        edition: "0.1",
        meaning: "C member error: undeclared enum, argument-less enum variant, or missing struct field",
    },
    CatalogEntry {
        code: "NS8021",
        edition: "0.1",
        meaning: "C `for`-loop iteration is limited to arrays, maps, and bytes in the first slice",
    },
    CatalogEntry {
        code: "NS8022",
        edition: "0.1",
        meaning: "Fixed-size arrays are not supported by the C backend first slice",
    },
    CatalogEntry {
        code: "NS8024",
        edition: "0.1",
        meaning: "Printing this value type is not supported by the C backend first slice",
    },
    CatalogEntry {
        code: "NS8025",
        edition: "0.1",
        meaning: "The tuple type is unavailable for this tuple expression in the C backend",
    },
    CatalogEntry {
        code: "NS8099",
        edition: "0.1",
        meaning: "`Never` appears as a stored type, which the C backend cannot emit",
    },
    CatalogEntry {
        code: "NS9000",
        edition: "0.1",
        meaning: "Unexpected CLI or tooling failure; not a source diagnostic",
    },
    CatalogEntry {
        code: "NS9001",
        edition: "0.1",
        meaning: "No manifest found; exactly one `core.json` or `core.toml` is required",
    },
    CatalogEntry {
        code: "NS9002",
        edition: "0.1",
        meaning: "Unable to read or parse a manifest",
    },
    CatalogEntry {
        code: "NS9003",
        edition: "0.1",
        meaning: "Compiler binary not found",
    },
    CatalogEntry {
        code: "NS9004",
        edition: "0.1",
        meaning: "Compiler process failed",
    },
    CatalogEntry {
        code: "NS9005",
        edition: "0.1",
        meaning: "Compiler returned an invalid, incomplete, or incompatible response",
    },
    CatalogEntry {
        code: "NS9006",
        edition: "0.1",
        meaning: "JSON-RPC request failed",
    },
    CatalogEntry {
        code: "NS9007",
        edition: "0.1",
        meaning: "Source or module is not valid UTF-8",
    },
    CatalogEntry {
        code: "NS9008",
        edition: "0.1",
        meaning: "A project or source path cannot be resolved, read, or accessed",
    },
    CatalogEntry {
        code: "NS9009",
        edition: "0.1",
        meaning: "Unsupported compiler API version",
    },
    CatalogEntry {
        code: "NS9010",
        edition: "0.1",
        meaning: "A compiler request contains no source units",
    },
    CatalogEntry {
        code: "NS9011",
        edition: "0.1",
        meaning: "The build target is missing or not declared in the manifest",
    },
    CatalogEntry {
        code: "NS9012",
        edition: "0.1",
        meaning: "A build configuration value is invalid: target mode, integer policy, or emit kind",
    },
    CatalogEntry {
        code: "NS9013",
        edition: "0.1",
        meaning: "The entry point is missing, outside the project, or not a `.ns` file",
    },
    CatalogEntry {
        code: "NS9014",
        edition: "0.1",
        meaning: "A build output directory or file cannot be created or written",
    },
    CatalogEntry {
        code: "NS9015",
        edition: "0.1",
        meaning: "A native compile, link, or archive step failed",
    },
    CatalogEntry {
        code: "NS9016",
        edition: "0.1",
        meaning: "A path dependency is missing or escapes its package or project root",
    },
    CatalogEntry {
        code: "NS9101",
        edition: "0.1",
        meaning: "The build manifest or `toolchain` section is malformed",
    },
    CatalogEntry {
        code: "NS9102",
        edition: "0.1",
        meaning: "No cached or `PATH` toolchain was found for the requested family",
    },
    CatalogEntry {
        code: "NS9103",
        edition: "0.1",
        meaning: "`ASTRA_CC` does not exist or is not a usable compiler",
    },
    CatalogEntry {
        code: "NS9999",
        edition: "0.1",
        meaning: "Reserved sentinel; tests assert it never appears in output",
    },
    CatalogEntry {
        code: "NV0000",
        edition: "0.1",
        meaning: "Frontend diagnostics were returned unchanged",
    },
    CatalogEntry {
        code: "NV0001",
        edition: "0.1",
        meaning: "Manual memory is unsupported by the first Nova engine",
    },
    CatalogEntry {
        code: "NV0002",
        edition: "0.1",
        meaning: "Missing entry point",
    },
    CatalogEntry {
        code: "NV0003",
        edition: "0.1",
        meaning: "Unsupported runtime feature or host adapter",
    },
    CatalogEntry {
        code: "NV0004",
        edition: "0.1",
        meaning: "Unhandled error escaped the entry point",
    },
    CatalogEntry {
        code: "NV0005",
        edition: "0.1",
        meaning: "Panic, including the interpreted call stack",
    },
    CatalogEntry {
        code: "NV0006",
        edition: "0.1",
        meaning: "Abort without unwinding",
    },
    CatalogEntry {
        code: "NV0007",
        edition: "0.1",
        meaning: "Nova internal invariant failure",
    },
    CatalogEntry {
        code: "NV0008",
        edition: "0.1",
        meaning: "Foreign script requires a production target",
    },
    CatalogEntry {
        code: "PG0001",
        edition: "0.1",
        meaning: "Protocol version mismatch between the bridge and the compiler",
    },
    CatalogEntry {
        code: "PG0002",
        edition: "0.1",
        meaning: "Wasm trap or panic inside the compiler",
    },
    CatalogEntry {
        code: "PG0003",
        edition: "0.1",
        meaning: "Malformed or truncated compiler response",
    },
    CatalogEntry {
        code: "PG0004",
        edition: "0.1",
        meaning: "Allocation failure inside the Wasm instance",
    },
];
