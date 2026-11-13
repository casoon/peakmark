// Runs one scenario against one module in a fresh Node process and reports
// the capacity of the module's linear memories after instantiation and after
// every run.
//
// peakmark passes the job as JSON in PEAKMARK_JOB and reads the line that
// starts with PEAKMARK_RESULT; anything else a scenario prints is ignored.

import { readFile } from "node:fs/promises";
import { syncBuiltinESMExports } from "node:module";
import { pathToFileURL } from "node:url";
import workerThreads from "node:worker_threads";

const job = JSON.parse(process.env.PEAKMARK_JOB);
const artifact = await readFile(job.artifact);

const OriginalModule = WebAssembly.Module;
const bytesOf = (source) =>
  ArrayBuffer.isView(source)
    ? Buffer.from(source.buffer, source.byteOffset, source.byteLength)
    : Buffer.from(source);

// Which WebAssembly.Module objects were compiled from the configured artifact.
// Instances of any other module are counted and their memory is left out.
const configured = new WeakMap();
const remember = (module, source) => {
  configured.set(module, bytesOf(source).equals(artifact));
  return module;
};

// Every memory of a configured instance, once, however many instances share it.
// Memories are held weakly: a memory the module and the scenario let go of is
// not kept alive for the sake of being measured.
const records = [];
const byMemory = new WeakMap();
let samples = 0;
let instances = 0;
let unrelated = 0;
let workers = 0;
const perInstance = [];

const record = (memory, origin) => {
  const known = byMemory.get(memory);
  if (known) {
    known.instances += 1;
    return 1;
  }
  const entry = {
    ref: new WeakRef(memory),
    origin,
    shared: memory.buffer instanceof SharedArrayBuffer,
    instances: 1,
    trace: Array(samples).fill(null),
  };
  byMemory.set(memory, entry);
  records.push(entry);
  return 1;
};

const watch = (instance, module, imports) => {
  if (!configured.get(module)) {
    unrelated += 1;
    return;
  }
  instances += 1;
  let seen = 0;
  for (const value of Object.values(instance.exports)) {
    if (value instanceof WebAssembly.Memory) seen += record(value, "export");
  }
  // A memory the module imports is observable through the imports it was given.
  for (const { module: namespace, name, kind } of OriginalModule.imports(module)) {
    const value = imports?.[namespace]?.[name];
    if (kind === "memory" && value instanceof WebAssembly.Memory) {
      seen += record(value, "import");
    }
  }
  perInstance.push(seen);
};

const instantiate = WebAssembly.instantiate;
WebAssembly.instantiate = async (source, imports) => {
  const result = await instantiate.call(WebAssembly, source, imports);
  if (result instanceof WebAssembly.Instance) {
    watch(result, source, imports);
  } else {
    remember(result.module, source);
    watch(result.instance, result.module, imports);
  }
  return result;
};
const compile = WebAssembly.compile;
WebAssembly.compile = async (source) =>
  remember(await compile.call(WebAssembly, source), source);
// Streaming takes a response; its bytes are read from a clone.
const bytesOfResponse = async (source) => {
  const response = await source;
  return [response, new Uint8Array(await response.clone().arrayBuffer())];
};
if (WebAssembly.instantiateStreaming) {
  const instantiateStreaming = WebAssembly.instantiateStreaming;
  WebAssembly.instantiateStreaming = async (source, imports) => {
    const [response, bytes] = await bytesOfResponse(source);
    const result = await instantiateStreaming.call(WebAssembly, response, imports);
    remember(result.module, bytes);
    watch(result.instance, result.module, imports);
    return result;
  };
}
if (WebAssembly.compileStreaming) {
  const compileStreaming = WebAssembly.compileStreaming;
  WebAssembly.compileStreaming = async (source) => {
    const [response, bytes] = await bytesOfResponse(source);
    return remember(await compileStreaming.call(WebAssembly, response), bytes);
  };
}
WebAssembly.Module = new Proxy(OriginalModule, {
  construct: (target, args, newTarget) =>
    remember(Reflect.construct(target, args, newTarget), args[0]),
});
WebAssembly.Instance = new Proxy(WebAssembly.Instance, {
  construct: (target, args, newTarget) => {
    const instance = Reflect.construct(target, args, newTarget);
    watch(instance, args[0], args[1]);
    return instance;
  },
});
// A module run in a worker has a memory of its own that is not seen here.
workerThreads.Worker = new Proxy(workerThreads.Worker, {
  construct: (target, args, newTarget) => {
    workers += 1;
    return Reflect.construct(target, args, newTarget);
  },
});
syncBuiltinESMExports();

// Reads the capacity of every memory still reachable and returns the sum of them.
const sample = () => {
  let bytes = 0;
  for (const entry of records) {
    const memory = entry.ref.deref();
    const held = memory ? memory.buffer.byteLength : null;
    entry.trace.push(held);
    bytes += held ?? 0;
  }
  samples += 1;
  return bytes;
};

try {
  const started = performance.now();
  let module;
  if (job.glue) {
    const glue = await import(pathToFileURL(job.glue).href);
    if (typeof glue.default === "function") {
      // wasm-bindgen --target web: the default export instantiates.
      await glue.default({ module_or_path: artifact });
      module = glue;
    } else {
      module = glue.default ?? glue;
    }
  } else {
    module = (await WebAssembly.instantiate(artifact, {})).instance.exports;
  }
  const instantiateMs = performance.now() - started;

  if (instances === 0) {
    throw new Error(
      "no instance of the module was observed: it is instantiated by a path peakmark does not watch",
    );
  }
  if (records.length === 0) {
    throw new Error(
      "no memory of the module can be observed: it neither exports nor imports one, so there is nothing to measure",
    );
  }

  const scenario = await import(pathToFileURL(job.scenario).href);
  if (typeof scenario.default !== "function") {
    throw new Error("the scenario has no default export that is a function");
  }

  const afterInstantiate = sample();
  const afterRun = [];
  for (let run = 0; run < job.repeat; run++) {
    await scenario.default(module);
    afterRun.push(sample());
  }

  // Exit once the line is written, so a timer or socket the scenario left
  // behind does not keep this process, and with it the check, waiting.
  const result = JSON.stringify({
    afterInstantiate,
    afterRun,
    instantiateMs,
    node: process.version,
    memories: records.map((entry) => ({
      origin: entry.origin,
      shared: entry.shared,
      instances: entry.instances,
      afterInstantiate: entry.trace[0],
      afterRun: entry.trace.slice(1),
    })),
    coverage: { instances, unrelated, workers, perInstance },
  });
  process.stdout.write("PEAKMARK_RESULT " + result + "\n", () => process.exit(0));
} catch (error) {
  // Exiting before the write is flushed would lose the message on a full pipe.
  process.stderr.write((error?.message ?? String(error)) + "\n", () => process.exit(1));
}
