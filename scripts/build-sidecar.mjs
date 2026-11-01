import { spawnSync } from 'node:child_process';
import { copyFileSync, mkdirSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

const root = fileURLToPath(new URL('../', import.meta.url));
const rustc = spawnSync('rustc', ['-vV'], { encoding: 'utf8' });
if (rustc.status !== 0) throw new Error(rustc.stderr || '找不到 Rust 工具链');
const target = process.env.TAURI_ENV_TARGET_TRIPLE || rustc.stdout.match(/^host: (.+)$/m)?.[1];
if (!target || !/^[a-zA-Z0-9_.-]+$/.test(target)) throw new Error('无法确定 Rust target');
const result = spawnSync('cargo', [
  'build', '--release', '--locked', '--manifest-path', path.join(root, 'sidecar/Cargo.toml'), '--target', target,
], { stdio: 'inherit', cwd: root });
if (result.status !== 0) process.exit(result.status ?? 1);
const suffix = target.includes('windows') ? '.exe' : '';
const destination = path.join(root, 'src-tauri/binaries', `dsh-sidecar-${target}${suffix}`);
mkdirSync(path.dirname(destination), { recursive: true });
copyFileSync(path.join(root, 'sidecar/target', target, 'release', `dsh-sidecar${suffix}`), destination);
console.log(`Sidecar: ${destination}`);
