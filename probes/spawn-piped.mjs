// Imitates how the VS Code extension starts Claude Code: piped stdio, hidden window, no terminal.
//
// Usage: node spawn-piped.mjs <claude> [args...]
//
// Prints the child's PID as its first line and "exit <code> <signal>" when the child ends, then
// keeps running so the pipes stay open until this launcher is stopped.
import { spawn } from 'node:child_process';

const [claude, ...args] = process.argv.slice(2);
const child = spawn(claude, args, { stdio: ['pipe', 'pipe', 'pipe'], windowsHide: true });

child.once('spawn', () => console.log(child.pid));
child.once('error', (error) => {
  console.log(`error ${error.message}`);
  process.exit(1);
});
child.once('exit', (code, signal) => console.log(`exit ${code ?? ''} ${signal ?? ''}`.trimEnd()));

// Nobody reads the output; drain it so the pipes never fill up and block the child.
child.stdout.resume();
child.stderr.resume();

setInterval(() => {}, 1 << 30);
