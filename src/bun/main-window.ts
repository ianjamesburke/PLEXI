import { BrowserWindow, Screen } from "electrobun/bun";

function getDefaultFrame() {
  const primaryDisplay = Screen.getPrimaryDisplay();
  const workArea = primaryDisplay.workArea;
  const width = Math.round(workArea.width * 0.8);
  const height = Math.round(workArea.height * 0.8);

  if (process.platform === "darwin") {
    return {
      width,
      height,
      x: 48,
      y: 48,
    };
  }

  return {
    width,
    height,
    x: 100,
    y: 100,
  };
}

export function createMainWindow(rpc: unknown) {
  const frame = getDefaultFrame();

  return new BrowserWindow({
    title: "Plexi",
    url: "views://mainview/index.html",
    rpc,
    titleBarStyle: process.platform === "darwin" ? "hiddenInset" : "default",
    frame,
  });
}
