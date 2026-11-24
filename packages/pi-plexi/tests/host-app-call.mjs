// Rust's host harness services this Pi tool's CLI requests and inspects both
// continuations before returning each response (permission_gate_tests.rs).
import { createInterface } from "node:readline";
import { PlexiCliClient } from "../src/cli-client.ts";
import { buildPlexiTools } from "../src/tools.ts";

const input = JSON.parse(process.argv[2]);
const replies = createInterface({ input: process.stdin });
const lines = replies[Symbol.asyncIterator]();
const runner = async (_binary, args) => {
  process.stdout.write(JSON.stringify(args) + "\n");
  const reply = await lines.next();
  if (reply.done) throw Error("host harness closed before replying");
  return { exitCode: 0, stdout: reply.value, stderr: "" };
};
try {
  const client = new PlexiCliClient({ attached: true }, runner);
  const tool = buildPlexiTools({ client }).find((tool) => tool.name === "plexi_app_call");
  const result = await tool.execute("pi-turn", { app_id: "chess", tool: "chess.play", args: input }, undefined, undefined, {});
  if (result.isError) throw Error(JSON.stringify(result));
} finally {
  replies.close();
}
