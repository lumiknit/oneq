import type { Component } from 'solid-js';
import { createEffect, createSignal } from 'solid-js';
import CLIOptionUI from './CLIOptionUI';
import {
  parseCLIFormState,
  serializeCLIFormState,
  type CLIFormState,
} from './options';

type CLIOptionProps = {
  options: () => string;
  setOptions: (next: string) => void;
  disabled?: boolean;
};

/** CLI-options editor: a "Raw" tab (free-form argv text, default) and a
 * "Form" tab (structured). Both tabs stay in sync
 * through `options`/`setOptions` so switching never loses input. */
const CLIOption: Component<CLIOptionProps> = (props) => {
  const [mode, setMode] = createSignal<'form' | 'raw'>('raw');
  const [form, setForm] = createSignal<CLIFormState>(
    parseCLIFormState(props.options()),
  );

  let oldOption: string = '';

  createEffect(() => {
    let newOpt = props.options();
    if (oldOption !== newOpt) {
      oldOption = newOpt;
      setForm(parseCLIFormState(newOpt));
    }
  });

  const switchTo = (next: 'form' | 'raw') => {
    if (next === mode()) return;
    if (next === 'form') setForm(parseCLIFormState(props.options()));
    setMode(next);
  };

  const onFormChange = (next: CLIFormState) => {
    setForm(next);
    props.setOptions(serializeCLIFormState(next));
  };

  return (
    <div class="cli-option">
      <div class="cli-option-tabs" role="group" aria-label="CLI options mode">
        <button
          type="button"
          role="tab"
          aria-selected={mode() === 'raw'}
          class={mode() === 'raw' ? 'primary' : 'secondary'}
          classList={{ active: mode() === 'raw' }}
          disabled={props.disabled}
          onClick={() => switchTo('raw')}
        >
          Raw
        </button>
        <button
          type="button"
          role="tab"
          aria-selected={mode() === 'form'}
          class={mode() === 'form' ? 'primary' : 'secondary'}
          classList={{ active: mode() === 'form' }}
          disabled={props.disabled}
          onClick={() => switchTo('form')}
        >
          Form
        </button>
      </div>
      {mode() === 'form' ? (
        <CLIOptionUI
          state={form()}
          onChange={onFormChange}
          disabled={props.disabled}
        />
      ) : (
        <input
          type="text"
          value={props.options()}
          onInput={(e) => props.setOptions(e.currentTarget.value)}
          placeholder={'-c --arg name "hello world"'}
          spellcheck={false}
          autocomplete="off"
          disabled={props.disabled}
        />
      )}
    </div>
  );
};

export default CLIOption;
