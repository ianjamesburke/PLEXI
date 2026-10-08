# Assistant pane UI

Design note for the Assistant transcript, composer, and the shared list rows the slash menu and command palette both use. This is not a PRM: the agent loop, history, tool dispatch, and broker stay behind `AssistantModel`.

## Layout

The hint bar and composer are placed from the pane floor using heights measured on the current frame. The transcript occupies whatever space remains above them. The slash menu and `/model` overlay stay floating popups anchored to the composer, so opening or filtering them does not reflow the transcript.

Before: the composer, permission sheet, and hint bar shared a content-sized bottom panel. That panel clips to last frame's height, so Shift+Enter hid the hint bar for one frame, and the composer's own scroll viewport lagged the same way. After: the hint slot's height ignores the composer, the composer grows upward, and the text viewport is pinned to the height measured before it is shown.

## Transcript

A fresh conversation, a sent message, and a streaming reply stay pinned to the latest line. A wheel toward older messages, or a scrollbar drag that leaves the bottom, releases the pin and shows a Latest control. Choosing it pins again. Sending a message pins again even if the reader had scrolled up.

Before: `stick_to_bottom` dropped the pin on the first frame the content outgrew the viewport, and nothing offered a way back. After: follow state is explicit. While pinned, a pass that painted above the bottom stores the measured end offset and repaints that same frame, so the latest lines stay inside the pane. The jump control appears only when the reader is above the latest line.

## Messages

Consecutive turns from the same role share one caption and a tighter gap. User turns read "You" and stay right-aligned. Assistant, slash output, and errors use a left caption ("Assistant" or "Error") and a bubble or danger-colored body. A streaming reply is one left bubble from the first token; the thinking beat sits inside that bubble until a tool is actually running.

A finished tool call is a collapsing row. The header is the status mark, the tool name, and the first line of the input summary. The body keeps the input, output, and diff blocks. Failures start open. A row with none of those payloads stays a single line. "Review permissions" is unchanged.

## Menus

`/model` labels each tier with the configured model id for the active backend, for example `Medium — xiaomi/mimo-v2.5-pro`. The id is view data filled when the picker opens. A tier with no id stays `Low`, `Medium`, or `High`.

`ListRow` now reserves a few pixels under every row, paints a hairline in that gap, and strokes the row with the accent color on hover. The command palette, the slash menu, and the model picker share that row, so the separation shows up in all three. The hover stroke matters on the assistant popup, whose fill matches the row's hover fill.

## Next

- Stream the caret at the end of the partial reply so the bubble reads as live text, not a block that replaces itself.
- Group a tool call under the assistant turn that issued it, instead of as a sibling row with its own caption rhythm.
- Remember which tool rows the reader opened across a conversation switch.
- Give the composer an explicit model chip in the field, not only inside `/model`, once the harness exposes the resolved id on the model.
