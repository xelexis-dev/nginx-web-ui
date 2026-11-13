import { directivesToState, type DirectiveEditorState } from "./directive-state";
import type { Directive } from "./types";

/** null is stored absence; undefined is a new editor with nothing to carry. */
export type OriginalDirectives = Directive[] | null | undefined;
export type DirectiveSubmission = Directive[] | DirectiveEditorState | null;

function fields(state: DirectiveEditorState): unknown[] {
  return [
    state.headers.map(({ name, value }) => [name, value]),
    state.timeouts.read, state.timeouts.send, state.timeouts.connect,
    state.bodySize,
    state.addHeaders.map(({ name, value }) => [name, value]),
    state.rewrites.map(({ from, to, flag }) => [from, to, flag]),
    state.httpVersion, state.buffering, state.advanced,
  ];
}

export function directiveStateUnchanged(original: OriginalDirectives, current: DirectiveEditorState): boolean {
  return original !== undefined && JSON.stringify(fields(current)) === JSON.stringify(fields(directivesToState(original ?? [])));
}

/** Never rebuild an untouched set: order, quoting inputs and missing-key semantics affect nginx bytes. */
export function carryDirectives(original: OriginalDirectives, current: DirectiveEditorState): DirectiveSubmission {
  return original !== undefined && directiveStateUnchanged(original, current) ? original : current;
}
