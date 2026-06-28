// Temporary bridge so extracted modules keep working before the IPC cutover (Task 7).
export const invoke = (cmd: string, args?: Record<string, unknown>): Promise<any> =>
  (window as any).__TAURI__.core.invoke(cmd, args);
