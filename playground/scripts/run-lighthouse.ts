let failed = false;
for (const config of ['.lighthouserc.cjs', '.lighthouserc.mobile.cjs']) {
  const process = Bun.spawn(
    ['bun', 'x', 'lhci', 'autorun', '--config', config],
    {
      stdin: 'inherit',
      stdout: 'inherit',
      stderr: 'inherit',
    },
  );
  const exitCode = await process.exited;
  if (exitCode !== 0) failed = true;
}

if (failed) globalThis.process.exit(1);

export {};
