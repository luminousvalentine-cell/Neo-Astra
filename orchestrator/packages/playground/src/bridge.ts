export const SUPPORTED_PROTOCOL_VERSION = 1;

export type BridgeErrorCode =
  | "PG0001"
  | "PG0002"
  | "PG0003"
  | "PG0004";

export class CompilerBridgeError extends Error {

  readonly code: BridgeErrorCode;

  constructor(code: BridgeErrorCode, message: string, options?: { cause?: unknown }) {
    super(message, options as ErrorOptions | undefined);
    this.name = "CompilerBridgeError";
    this.code = code;
  }
}

export class CompilerResponseError extends Error {

  readonly code: number;

  readonly data: unknown;

  constructor(code: number, message: string, data?: unknown) {
    super(message);
    this.name = "CompilerResponseError";
    this.code = code;
    this.data = data;
  }
}

export interface JsonRpcRequest<Params = unknown> {
  jsonrpc: "2.0";
  id: number | string;
  method: string;
  params: Params;
}

export interface JsonRpcSuccess<Result = unknown> {
  jsonrpc: "2.0";
  id: number | string;
  result: Result;
}

export interface JsonRpcFailure {
  jsonrpc: "2.0";
  id: number | string | null;
  error: {
    code: number;
    message: string;
    data?: unknown;
  };
}

export type JsonRpcResponse<Result = unknown> = JsonRpcSuccess<Result> | JsonRpcFailure;

interface CompilerWasmExports {
  readonly memory: WebAssembly.Memory;
  astra_protocol_version(): number;
  astra_alloc(len: number): number;
  astra_free(ptr: number, len: number): void;
  astra_compile(ptr: number, len: number): bigint;
}

export interface CompilerBridgeOptions {

  expectedProtocolVersion?: number;
}

const REQUIRED_EXPORTS = [
  "memory",
  "astra_protocol_version",
  "astra_alloc",
  "astra_free",
  "astra_compile",
] as const;

function readExports(exports: WebAssembly.Exports): CompilerWasmExports {
  for (const name of REQUIRED_EXPORTS) {
    if (!(name in exports)) {
      throw new CompilerBridgeError(
        "PG0003",
        `Wasm module is missing the '${name}' export; rebuild crates/astra-wasm`,
      );
    }
  }

  return exports as unknown as CompilerWasmExports;
}

export class CompilerBridge {
  readonly #exports: CompilerWasmExports;
  readonly #encoder = new TextEncoder();
  readonly #decoder = new TextDecoder("utf-8", { fatal: true });
  #closed = false;

  private constructor(exports: CompilerWasmExports) {
    this.#exports = exports;
  }

  static async instantiate(
    source: BufferSource | WebAssembly.Module,
    options: CompilerBridgeOptions = {},
  ): Promise<CompilerBridge> {
    let module: WebAssembly.Module;
    try {
      module =
        source instanceof WebAssembly.Module ? source : await WebAssembly.compile(source);
    } catch (cause) {
      throw new CompilerBridgeError("PG0002", "Failed to compile the compiler Wasm module", {
        cause,
      });
    }

    let instance: WebAssembly.Instance;
    try {
      instance = await WebAssembly.instantiate(module);
    } catch (cause) {
      throw new CompilerBridgeError("PG0002", "Failed to instantiate the compiler Wasm module", {
        cause,
      });
    }

    const exports = readExports(instance.exports);
    const expected = options.expectedProtocolVersion ?? SUPPORTED_PROTOCOL_VERSION;
    const actual = exports.astra_protocol_version();
    if (actual !== expected) {
      throw new CompilerBridgeError(
        "PG0001",
        `Compiler protocol version ${actual} does not match host version ${expected}`,
      );
    }

    return new CompilerBridge(exports);
  }

  get protocolVersion(): number {
    return this.#exports.astra_protocol_version();
  }

  get closed(): boolean {
    return this.#closed;
  }

  async request<Result = unknown, Params = unknown>(
    request: JsonRpcRequest<Params>,
  ): Promise<Result> {
    const text = JSON.stringify(request);
    const raw = this.#exchange(text);
    const response = this.#parse(raw);

    if ("error" in response) {
      throw new CompilerResponseError(
        response.error.code,
        response.error.message,
        response.error.data,
      );
    }

    if (response.id !== request.id) {
      throw new CompilerBridgeError(
        "PG0003",
        `Compiler answered request '${String(response.id)}' while '${String(request.id)}' was pending`,
      );
    }

    return response.result as Result;
  }

  close(): void {
    this.#closed = true;
  }

  #exchange(text: string): string {
    if (this.#closed) {
      throw new CompilerBridgeError("PG0003", "Compiler bridge is closed");
    }

    const input = this.#encoder.encode(text);
    const { astra_alloc, astra_free, astra_compile, memory } = this.#exports;

    let inputPointer = 0;
    try {
      if (input.byteLength > 0) {
        inputPointer = astra_alloc(input.byteLength);
        if (inputPointer === 0) {
          throw new CompilerBridgeError("PG0004", "astra_alloc returned a null pointer");
        }

        new Uint8Array(memory.buffer, inputPointer, input.byteLength).set(input);
      }

      let packed: bigint;
      try {
        packed = BigInt(astra_compile(inputPointer, input.byteLength));
      } catch (cause) {
        throw new CompilerBridgeError("PG0002", "The compiler trapped while handling a request", {
          cause,
        });
      }

      const outputPointer = Number(packed >> 32n);
      const outputLength = Number(packed & 0xffff_ffffn);

      if (outputPointer === 0 || outputLength === 0) {
        throw new CompilerBridgeError("PG0003", "The compiler returned an empty response");
      }

      const buffer = memory.buffer;
      if (outputPointer + outputLength > buffer.byteLength) {
        throw new CompilerBridgeError(
          "PG0003",
          "The compiler response points outside linear memory",
        );
      }

      try {
        return this.#decoder.decode(new Uint8Array(buffer, outputPointer, outputLength));
      } catch (cause) {
        throw new CompilerBridgeError("PG0003", "The compiler response is not valid UTF-8", {
          cause,
        });
      } finally {
        astra_free(outputPointer, outputLength);
      }
    } finally {
      if (inputPointer !== 0 && input.byteLength > 0) {
        astra_free(inputPointer, input.byteLength);
      }
    }
  }

  #parse(text: string): JsonRpcResponse {
    let value: unknown;
    try {
      value = JSON.parse(text);
    } catch (cause) {
      throw new CompilerBridgeError("PG0003", "The compiler response is not valid JSON", {
        cause,
      });
    }

    if (
      typeof value !== "object" ||
      value === null ||
      (value as { jsonrpc?: unknown }).jsonrpc !== "2.0"
    ) {
      throw new CompilerBridgeError("PG0003", "The compiler response is not JSON-RPC 2.0");
    }

    const response = value as JsonRpcResponse;
    if ("error" in response) {
      const error = response.error;
      if (typeof error !== "object" || error === null || typeof error.code !== "number") {
        throw new CompilerBridgeError("PG0003", "The compiler returned a malformed JSON-RPC error");
      }
      return response;
    }

    if (!("result" in response)) {
      throw new CompilerBridgeError("PG0003", "The compiler response has no result");
    }

    return response;
  }
}
