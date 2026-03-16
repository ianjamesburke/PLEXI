import type { ElectrobunConfig } from "electrobun";

const NATIVE_COPY_TARGETS = [
	"node_modules/bun-pty/rust-pty/target/release/librust_pty_arm64.dylib",
	"node_modules/bun-pty/rust-pty/target/release/librust_pty.dylib",
	"node_modules/bun-pty/rust-pty/target/release/librust_pty_arm64.so",
	"node_modules/bun-pty/rust-pty/target/release/librust_pty.so",
	"node_modules/bun-pty/rust-pty/target/release/rust_pty.dll",
] as const;

function filename(pathname: string) {
	return pathname.split("/").pop() || pathname;
}

function buildCopyMap() {
	return {
		"src/mainview/index.html": "views/mainview/index.html",
		"src/mainview/index.css": "views/mainview/index.css",
		"src/mainview/app.js": "views/mainview/app.js",
		"src/mainview/assets/fonts/JetBrainsMono-Regular.otf": "views/mainview/assets/fonts/JetBrainsMono-Regular.otf",
		"src/mainview/assets/fonts/JetBrainsMono-SemiBold.otf": "views/mainview/assets/fonts/JetBrainsMono-SemiBold.otf",
		"src/shared/workspace-state.js": "views/shared/workspace-state.js",
		"src/shared/workspace-document.js": "views/shared/workspace-document.js",
		"node_modules/@xterm/xterm/lib/xterm.js": "views/mainview/vendor/xterm/xterm.js",
		"node_modules/@xterm/xterm/lib/xterm.js.map": "views/mainview/vendor/xterm/xterm.js.map",
		"node_modules/@xterm/xterm/css/xterm.css": "views/mainview/vendor/xterm/xterm.css",
		"node_modules/@xterm/addon-fit/lib/addon-fit.js": "views/mainview/vendor/xterm/addon-fit.js",
		"node_modules/@xterm/addon-fit/lib/addon-fit.js.map": "views/mainview/vendor/xterm/addon-fit.js.map",
		"node_modules/@xterm/addon-web-links/lib/addon-web-links.js": "views/mainview/vendor/xterm/addon-web-links.js",
		"node_modules/@xterm/addon-web-links/lib/addon-web-links.js.map": "views/mainview/vendor/xterm/addon-web-links.js.map",
		...Object.fromEntries(
			NATIVE_COPY_TARGETS.map((source) => [source, `native/${filename(source)}`]),
		),
	};
}

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
		copy: buildCopyMap(),
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
