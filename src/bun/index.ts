import { BrowserWindow } from "electrobun/bun";

// Create the main application window
const mainWindow = new BrowserWindow({
  title: "Plexi",
  url: "views://mainview/index.html",
  frame: {
    width: 1200,
    height: 800,
    x: 100,
    y: 100,
  },
});

console.log("Plexi started!");
