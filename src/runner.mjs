// Runs one scenario against one module in a fresh Node process and reports
// the module's linear memory after instantiation and after every run.
//
// peakmark passes the job as JSON in PEAKMARK_JOB and reads the line that
// starts with PEAKMARK_RESULT; anything else a scenario prints is ignored.

import { readFile } from "node:fs/promises";
import { pathToFileURL } from "node:url";

const job = JSON.parse(process.env.PEAKMARK_JOB);

// Every instance created from here on is watched, whoever creates it: the
// glue, the scenario or this file. Memory is read from the instance's exports.
const memories = new Set();
const watch = (instance) => {
  for (const value of Object.values(instance.exports)) {
    if (value instanceof WebAssembly.Memory) memories.add(value);
  }
  return instance;
};

const instantiate = WebAssembly.instantiate;
WebAssembly.instantiate = async (source, imports) => {
  const result = await instantiate.call(WebAssembly, source, imports);
  watch(result instanceof WebAssembly.Instance ? result : result.instance);
  return result;
};
WebAssembly.Instance = new Proxy(WebAssembly.Instance, {
  construct: (target, args, newTarget) =>
    watch(Reflect.construct(target, args, newTarget)),
});

const held = () => {
  let bytes = 0;
  for (const memory of memories) bytes += memory.buffer.byteLength;
  return bytes;
};

try {
  const bytes = await readFile(job.artifact);
  const started = performance.now();
  let module;
  if (job.glue) {
    const glue = await import(pathToFileURL(job.glue).href);
    if (typeof glue.default === "function") {
      // wasm-bindgen --target web: the default export instantiates.
      await glue.default({ module_or_path: bytes });
      module = glue;
    } else {
      module = glue.default ?? glue;
    }
  } else {
    module = (await WebAssembly.instantiate(bytes, {})).instance.exports;
  }
  const instantiateMs = performance.now() - started;

  if (memories.size === 0) {
    throw new Error("the module exports no memory, so there is nothing to measure");
  }

  const scenario = await import(pathToFileURL(job.scenario).href);
  if (typeof scenario.default !== "function") {
    throw new Error("the scenario has no default export that is a function");
  }

  const afterInstantiate = held();
  const afterRun = [];
  for (let run = 0; run < job.repeat; run++) {
    await scenario.default(module);
    afterRun.push(held());
  }

  // Exit once the line is written, so a timer or socket the scenario left
  // behind does not keep this process, and with it the check, waiting.
  const result = JSON.stringify({ afterInstantiate, afterRun, instantiateMs });
  process.stdout.write("PEAKMARK_RESULT " + result + "\n", () => process.exit(0));
} catch (error) {
  // Exiting before the write is flushed would lose the message on a full pipe.
  process.stderr.write((error?.message ?? String(error)) + "\n", () => process.exit(1));
}
