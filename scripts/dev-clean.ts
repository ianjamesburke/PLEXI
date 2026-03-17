const child = Bun.spawn(
  [process.execPath, "x", "tauri", "dev"],
  {
    stdio: ["inherit", "inherit", "inherit"],
    env: {
      ...process.env,
      PLEXI_CLEAN: "1",
    },
  },
);

process.exit(await child.exited);
