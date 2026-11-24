"use strict";

// Execute the repository's actual Templater program with in-memory app/API
// doubles. This is a test adapter, not an Obsidian/Templater implementation.
// The only host subprocess serializes YAML through the test Python runtime.
const fs = require("node:fs");
const vm = require("node:vm");
const { spawnSync } = require("node:child_process");

async function main() {
  const input = JSON.parse(fs.readFileSync(0, "utf8"));
  if (!input.template.startsWith("<%*") || !/%>\s*$/.test(input.template)) {
    throw new Error("Expected one Templater execution block");
  }
  const events = { prompts: 0, requests: [], renames: [] };
  const yaml = (operation, value) => {
    const result = spawnSync(input.python, [input.bridge], {
      input: JSON.stringify({ operation, value }),
      encoding: "utf8",
      timeout: 10000,
      windowsHide: true,
    });
    if (result.error) throw result.error;
    if (result.status !== 0) throw new Error(result.stderr || "YAML test adapter failed");
    return JSON.parse(result.stdout);
  };
  const http = {
    get(options, callback) {
      if (options.hostname !== "127.0.0.1" || options.port !== 23119 ||
          options.path !== "/api/users/0/items/TEST0001") {
        throw new Error("Unexpected request from template");
      }
      events.requests.push({ hostname: options.hostname, port: options.port, path: options.path });
      const handlers = {};
      const response = {
        statusCode: 200,
        setEncoding(encoding) {
          if (encoding !== "utf8") throw new Error("Unexpected response encoding");
        },
        on(event, handler) {
          handlers[event] = handler;
          return response;
        },
      };
      callback(response);
      queueMicrotask(() => {
        handlers.data(JSON.stringify({ data: input.item }));
        handlers.end();
      });
      const request = { on() { return request; }, setTimeout() { return request; } };
      return request;
    },
  };
  const replies = ["TEST0001", JSON.stringify(input.profile), input.body];
  const context = vm.createContext({
    tp: {
      file: {
        content: "",
        async rename(name) { events.renames.push(name); },
      },
      config: { target_file: {
        path: "文献/template/临时运行测试/input.md",
        basename: "input",
        parent: { path: "文献/template/临时运行测试" },
      } },
      app: { vault: {
        getMarkdownFiles() { return []; },
        getAbstractFileByPath() { return null; },
      } },
      system: { async prompt() {
        if (events.prompts >= replies.length) throw new Error("Unexpected extra template prompt");
        return replies[events.prompts++];
      } },
      obsidian: {
        parseYaml: value => yaml("parse", value),
        stringifyYaml: value => yaml("stringify", value),
      },
    },
    window: { require(name) {
      if (name !== "http") throw new Error("Unexpected module request from template");
      return http;
    } },
  });
  const program = input.template.slice(3).replace(/%>\s*$/, "");
  const script = new vm.Script(
    `(async () => { let tR = "";\n${program}\nreturn tR; })()`,
    { filename: input.templateName },
  );
  const markdown = await script.runInContext(context, { timeout: 5000 });
  if (typeof markdown !== "string" || events.prompts !== 3) {
    throw new Error("Template did not complete its normal rendering path");
  }
  process.stdout.write(JSON.stringify({ markdown, events }));
}

main().catch(error => {
  process.stderr.write(`${error.stack || error}\n`);
  process.exitCode = 1;
});
