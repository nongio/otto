// pi's `question` tool for sessions Otto runs: the model asks the person to
// pick one of a few answers, and waits for it. Asked through pi's extension
// UI, which pi-acp turns into an ACP permission request whose options are the
// answers; Otto shows those as a choice, in the chat or in a dialog.
//
// pi-acp cancels free-text requests, so the answers are always the model's.

import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { Type } from "typebox";

const Params = Type.Object({
	question: Type.String({ description: "The question, as the person should read it" }),
	options: Type.Array(Type.String(), {
		minItems: 2,
		description: "The answers to pick from, each a few words",
	}),
});

export default function (pi: ExtensionAPI) {
	pi.registerTool({
		name: "question",
		label: "Question",
		description:
			"Ask the person to pick one of a few answers, and wait for it. Use it when you " +
			"cannot go on without their decision; for anything open-ended, ask in your reply.",
		parameters: Params,
		exposure: "model-only",
		executionMode: "sequential",

		async execute(_toolCallId, params, signal, _onUpdate, ctx) {
			if (!ctx.hasUI) {
				throw new Error("Nobody can be asked in this session; ask in your reply instead.");
			}
			const answer = await ctx.ui.select(params.question, params.options, { signal });
			const text =
				answer === undefined
					? "The person did not answer. Carry on without it, or ask in your reply."
					: `The person picked: ${answer}`;
			return {
				content: [{ type: "text", text }],
				details: { question: params.question, options: params.options, answer: answer ?? null },
			};
		},
	});
}
