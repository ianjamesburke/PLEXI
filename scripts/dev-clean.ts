const child = Bun.spawn(
  [process.execPath, "x", "electrobun", "dev", "--watch"],
  {
    stdio: ["inherit", "inherit", "inherit"],
    env: {
      ...process.env,
      PLEXI_CLEAN: "1",
    },
  },
);

process.exit(await child.exited);
