// A fixture's wasm32 build (`fixtures/wasm.rs`): the host writes an input into the
// instance's memory, runs the crate's entry on it, and copies the output back out.
// `differential.ts` runs it beside the translation, and `bench/time.ts` times each step.

interface Exports {
	memory: WebAssembly.Memory;
	input(len: number): number;
	run(): number;
	output(): number;
	message(): number;
	message_len(): number;
}

/**
 * How a call ended: its output, a panic with the message the module saved before it
 * trapped, or an abort, which trapped without running the panic hook.
 */
export type Outcome = Uint8Array | { panic: string } | "abort";

export class Wasm {
	#module: WebAssembly.Module;
	#exports: Exports;

	constructor(module: WebAssembly.Module) {
		this.#module = module;
		this.#exports = new WebAssembly.Instance(module, {}).exports as unknown as Exports;
	}

	/** Copy `input` into the instance, where the next `run` reads it. */
	input(input: Uint8Array) {
		const ptr = this.#exports.input(input.length);
		new Uint8Array(this.#exports.memory.buffer, ptr, input.length).set(input);
	}

	/** Run the entry on the last input, returning the output's length. A panic traps. */
	run(): number {
		return this.#exports.run();
	}

	/** Copy the last run's `len` bytes of output out of the instance. */
	output(len: number): Uint8Array {
		return new Uint8Array(this.#exports.memory.buffer, this.#exports.output(), len).slice();
	}

	/** All three, or how the entry trapped. */
	call(input: Uint8Array): Outcome {
		try {
			this.input(input);
			return this.output(this.run());
		} catch (err) {
			if (!(err instanceof WebAssembly.RuntimeError)) throw err;
			const ptr = this.#exports.message();
			const panic = () => new TextDecoder().decode(new Uint8Array(this.#exports.memory.buffer, ptr, this.#exports.message_len()));
			const outcome: Outcome = ptr === 0 ? "abort" : { panic: panic() };
			// A trap leaves the instance mid-call: start a fresh one.
			this.#exports = new WebAssembly.Instance(this.#module, {}).exports as unknown as Exports;
			return outcome;
		}
	}
}
