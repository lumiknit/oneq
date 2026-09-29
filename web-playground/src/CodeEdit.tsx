import type { Component } from 'solid-js';

type CodeEditProps = {
  value: string;
  onInput: (next: string) => void;
  id?: string;
  readOnly?: boolean;
  'aria-label'?: string;
};

const indentUnit = '  ';

/** Replaces the selection, preferring execCommand so undo history survives. */
const insertText = (el: HTMLTextAreaElement, text: string) => {
  el.focus();
  if (!document.execCommand('insertText', false, text)) {
    el.setRangeText(text, el.selectionStart, el.selectionEnd, 'end');
    el.dispatchEvent(new Event('input', { bubbles: true }));
  }
};

/** Indents or dedents every line touched by the selection. */
const shiftLines = (el: HTMLTextAreaElement, dedent: boolean) => {
  const { value, selectionStart, selectionEnd } = el;
  const lineStart = value.lastIndexOf('\n', selectionStart - 1) + 1;
  // A selection ending right after a newline does not include the next line.
  const endAt =
    selectionEnd > selectionStart && value[selectionEnd - 1] === '\n'
      ? selectionEnd - 1
      : selectionEnd;
  const nextNewline = value.indexOf('\n', endAt);
  const lineEnd = nextNewline < 0 ? value.length : nextNewline;
  const lines = value.slice(lineStart, lineEnd).split('\n');

  let firstDelta = 0;
  let totalDelta = 0;
  const shifted = lines.map((line, index) => {
    let next: string;
    if (dedent) {
      const remove = line.startsWith('\t')
        ? 1
        : Math.min(indentUnit.length, line.length - line.trimStart().length);
      next = line.slice(remove);
    } else {
      next = indentUnit + line;
    }
    const delta = next.length - line.length;
    if (index === 0) firstDelta = delta;
    totalDelta += delta;
    return next;
  });

  el.setSelectionRange(lineStart, lineEnd);
  insertText(el, shifted.join('\n'));
  el.setSelectionRange(
    Math.max(lineStart, selectionStart + firstDelta),
    Math.max(lineStart, selectionEnd + totalDelta),
  );
};

/** Caret offset under a drop point, or the current selection as a fallback. */
const dropOffset = (el: HTMLTextAreaElement, event: DragEvent) => {
  const doc = document as Document & {
    caretPositionFromPoint?: (
      x: number,
      y: number,
    ) => { offsetNode: Node; offset: number } | null;
  };
  const position = doc.caretPositionFromPoint?.(event.clientX, event.clientY);
  if (position && position.offsetNode === el) return position.offset;
  return el.selectionStart;
};

/** A textarea with editor-like keys (Tab, Shift+Tab, auto-indent on Enter)
 * and file drag & drop inserting the file's text at the drop position. */
const CodeEdit: Component<CodeEditProps> = (props) => {
  const onKeyDown = (
    event: KeyboardEvent & { currentTarget: HTMLTextAreaElement },
  ) => {
    const el = event.currentTarget;
    if (props.readOnly || event.isComposing) return;
    if (
      event.key === 'Tab' &&
      !event.ctrlKey &&
      !event.metaKey &&
      !event.altKey
    ) {
      event.preventDefault();
      const multiline = el.value
        .slice(el.selectionStart, el.selectionEnd)
        .includes('\n');
      if (event.shiftKey || multiline) shiftLines(el, event.shiftKey);
      else insertText(el, indentUnit);
    } else if (
      event.key === 'Enter' &&
      !event.shiftKey &&
      !event.ctrlKey &&
      !event.metaKey &&
      !event.altKey
    ) {
      event.preventDefault();
      const lineStart = el.value.lastIndexOf('\n', el.selectionStart - 1) + 1;
      const indent = /^[ \t]*/.exec(el.value.slice(lineStart))![0];
      insertText(el, `\n${indent.slice(0, el.selectionStart - lineStart)}`);
    }
  };

  const onDrop = async (
    event: DragEvent & { currentTarget: HTMLTextAreaElement },
  ) => {
    const file = event.dataTransfer?.files[0];
    if (!file || props.readOnly) return;
    event.preventDefault();
    const el = event.currentTarget;
    const offset = dropOffset(el, event);
    const text = await file.text();
    el.setSelectionRange(offset, offset);
    insertText(el, text);
  };

  return (
    <textarea
      id={props.id}
      value={props.value}
      readOnly={props.readOnly}
      aria-label={props['aria-label']}
      spellcheck={false}
      onInput={(event) => props.onInput(event.currentTarget.value)}
      onKeyDown={onKeyDown}
      onDragOver={(event) => {
        if (event.dataTransfer?.types.includes('Files')) event.preventDefault();
      }}
      onDrop={onDrop}
    />
  );
};

export default CodeEdit;
