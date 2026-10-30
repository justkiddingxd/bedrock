// Builds the bedrock-mcp program and puts it where Tauri picks up sidecars
// (src-tauri/binaries/bedrock-mcp-<target triple>[.exe]). The installer then
// ships it next to Bedrock.exe, where the app looks for it.
import { execFileSync } from 'node:child_process';
import { copyFileSync, mkdirSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const app = join(dirname(fileURLToPath(import.meta.url)), '..');
const root = join(app, '..');
const host = /host: (\S+)/.exec(execFileSync('rustc', ['-vV'], { encoding: 'utf8' }))[1];
// Set by the Tauri CLI when building for another target.
const triple = process.env.TAURI_ENV_TARGET_TRIPLE || host;
const exe = triple.includes('windows') ? '.exe' : '';

const args = ['build', '--release', '-p', 'bedrock-mcp'];
if (triple !== host) args.push('--target', triple);
execFileSync('cargo', args, { cwd: root, stdio: 'inherit' });

const target = process.env.CARGO_TARGET_DIR ? resolve(root, process.env.CARGO_TARGET_DIR) : join(root, 'target');
const built = join(target, ...(triple !== host ? [triple] : []), 'release', `bedrock-mcp${exe}`);
const dir = join(app, 'src-tauri', 'binaries');
mkdirSync(dir, { recursive: true });
copyFileSync(built, join(dir, `bedrock-mcp-${triple}${exe}`));
console.log(`sidecar: bedrock-mcp-${triple}${exe}`);
