import type { ElectrobunConfig } from "electrobun";

export default {
	app: {
		name: "plexi",
		identifier: "dev.plexi.app",
		version: "0.1.0",
	},
	build: {
		views: {
			mainview: {
				entrypoint: "src/mainview/app.js",
			},
		},
		copy: {
			"src/mainview/index.html": "views/mainview/index.html",
			"src/mainview/index.css": "views/mainview/index.css",
			"src/mainview/app.js": "views/mainview/app.js",
			"src/mainview/assets/fonts/JetBrainsMono-Regular.otf": "views/mainview/assets/fonts/JetBrainsMono-Regular.otf",
			"src/mainview/assets/fonts/JetBrainsMono-SemiBold.otf": "views/mainview/assets/fonts/JetBrainsMono-SemiBold.otf",
			"src/shared/workspace-state.js": "views/shared/workspace-state.js",
			"node_modules/xterm/lib/xterm.js": "views/mainview/vendor/xterm/xterm.js",
			"node_modules/xterm/css/xterm.css": "views/mainview/vendor/xterm/xterm.css",
			"node_modules/@xterm/addon-fit/lib/addon-fit.js": "views/mainview/vendor/xterm/addon-fit.js",
		},
		mac: {
			bundleCEF: false,
		},
		linux: {
			bundleCEF: false,
		},
		win: {
			bundleCEF: false,
		},
	},
} satisfies ElectrobunConfig;
