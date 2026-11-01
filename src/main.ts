import { invoke } from '@tauri-apps/api/core';
import './style.css';

type Status = { phase: 'starting' | 'running' | 'failed' | 'stopping'; message: string; logs: string[] };
const heading = document.querySelector('h1')!;
const message = document.querySelector<HTMLParagraphElement>('#message')!;
const spinner = document.querySelector<HTMLDivElement>('#spinner')!;
const retry = document.querySelector<HTMLButtonElement>('#retry')!;
const diagnostics = document.querySelector<HTMLDetailsElement>('#diagnostics')!;
const logs = document.querySelector<HTMLPreElement>('#logs')!;

async function refresh() {
  try {
    const status = await invoke<Status>('dsh_status');
    heading.textContent = status.phase === 'failed' ? 'DSH 启动或运行失败' : '正在启动 DSH';
    message.textContent = status.message;
    spinner.hidden = status.phase === 'failed';
    retry.hidden = status.phase !== 'failed';
    diagnostics.hidden = status.phase !== 'failed' || status.logs.length === 0;
    logs.textContent = status.logs.join('\n');
  } catch (error) {
    heading.textContent = '无法连接桌面服务';
    message.textContent = String(error);
    spinner.hidden = true;
  }
}

retry.addEventListener('click', async () => {
  retry.disabled = true;
  try {
    await invoke('restart_dsh');
  } catch (error) {
    message.textContent = String(error);
  } finally {
    retry.disabled = false;
    await refresh();
  }
});
void refresh();
setInterval(() => void refresh(), 500);
