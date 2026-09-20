import type { Component } from 'solid-js';
import { FORMAT_OPTIONS, type CLIFormState } from './options';

export type { CLIFormState };

type CLIOptionUIProps = {
  state: CLIFormState;
  onChange: (next: CLIFormState) => void;
  disabled?: boolean;
};

const CLIOptionUI: Component<CLIOptionUIProps> = (props) => {
  const set = <K extends keyof CLIFormState>(key: K, value: CLIFormState[K]) =>
    props.onChange({ ...props.state, [key]: value });

  return (
    <div class="cli-option-ui" aria-label="jq CLI options">
      <fieldset class="cli-option-row">
        <legend>Inputs</legend>
        <div class="grid">
          <label>
            <span>From</span>
            <select
              disabled={props.disabled}
              value={props.state.from}
              onChange={(e) => set('from', e.currentTarget.value)}
            >
              {FORMAT_OPTIONS.map((f) => (
                <option value={f.value}>{f.label}</option>
              ))}
            </select>
          </label>
          <label class="cli-option-check">
            <input
              type="checkbox"
              disabled={props.disabled}
              checked={props.state.rawInput}
              onChange={(e) => set('rawInput', e.currentTarget.checked)}
            />{' '}
            raw
          </label>
          <label class="cli-option-check">
            <input
              type="checkbox"
              disabled={props.disabled}
              checked={props.state.slurp}
              onChange={(e) => set('slurp', e.currentTarget.checked)}
            />{' '}
            slurp
          </label>
        </div>
      </fieldset>

      <fieldset class="cli-option-row">
        <legend>Outputs</legend>
        <div class="grid">
          <label>
            <span>To</span>
            <select
              disabled={props.disabled}
              value={props.state.to}
              onChange={(e) => set('to', e.currentTarget.value)}
            >
              {FORMAT_OPTIONS.map((f) => (
                <option value={f.value}>{f.label}</option>
              ))}
            </select>
          </label>
          <label>
            <span>Input</span>
            <select
              disabled={props.disabled}
              value={props.state.input}
              onChange={(e) =>
                set('input', e.currentTarget.value as CLIFormState['input'])
              }
            >
              <option value="stdin">stdin</option>
              <option value="null">null (-n)</option>
              <option value="doc">jq reference (--doc)</option>
            </select>
          </label>
          <label class="cli-option-check">
            <input
              type="checkbox"
              disabled={props.disabled}
              checked={props.state.tab}
              onChange={(e) => set('tab', e.currentTarget.checked)}
            />{' '}
            tabs
          </label>
          <label class="cli-option-check">
            <input
              type="checkbox"
              disabled={props.disabled}
              checked={props.state.ascii}
              onChange={(e) => set('ascii', e.currentTarget.checked)}
            />{' '}
            ascii-only
          </label>
          <label class="cli-option-check">
            <input
              type="checkbox"
              disabled={props.disabled}
              checked={props.state.sortKeys}
              onChange={(e) => set('sortKeys', e.currentTarget.checked)}
            />{' '}
            sort-keys
          </label>
        </div>
      </fieldset>

      <fieldset class="cli-option-row">
        <legend>Other</legend>
        <div class="grid">
          <label>
            <span>Stream</span>
            <select
              disabled={props.disabled}
              value={props.state.stream}
              onChange={(e) =>
                set('stream', e.currentTarget.value as CLIFormState['stream'])
              }
            >
              <option value="default">default</option>
              <option value="stream">stream (--stream)</option>
              <option value="stream-errors">
                stream + errors (--stream-errors)
              </option>
            </select>
          </label>
          <label>
            <span>Output string</span>
            <select
              disabled={props.disabled}
              value={props.state.outputMode}
              onChange={(e) =>
                set(
                  'outputMode',
                  e.currentTarget.value as CLIFormState['outputMode'],
                )
              }
            >
              <option value="default">default</option>
              <option value="raw">raw (-r)</option>
              <option value="raw0">raw + NUL (--raw-output0)</option>
              <option value="join">join, no newline (-j)</option>
            </select>
          </label>
          <label>
            <span>Layout</span>
            <select
              disabled={props.disabled}
              value={props.state.compact}
              onChange={(e) =>
                set('compact', e.currentTarget.value as CLIFormState['compact'])
              }
            >
              <option value="pretty">pretty</option>
              <option value="compact">compact (-c)</option>
              <option value="inline">inline (--inline-output)</option>
            </select>
          </label>
          <label>
            <span>Color</span>
            <select
              disabled={props.disabled}
              value={props.state.color}
              onChange={(e) =>
                set('color', e.currentTarget.value as CLIFormState['color'])
              }
            >
              <option value="auto">auto</option>
              <option value="color">on (-C)</option>
              <option value="mono">off (-M)</option>
            </select>
          </label>
          <label>
            <span>Indent</span>
            <input
              type="number"
              min="0"
              max="7"
              disabled={props.disabled || props.state.tab}
              value={props.state.indent}
              placeholder="2"
              onInput={(e) => set('indent', e.currentTarget.value)}
            />
          </label>
        </div>
      </fieldset>

      <div class="cli-option-flags cli-option-other-flags grid">
        <label>
          <input
            type="checkbox"
            disabled={props.disabled}
            checked={props.state.exitStatus}
            onChange={(e) => set('exitStatus', e.currentTarget.checked)}
          />
          exit status (-e)
        </label>
        <label>
          <input
            type="checkbox"
            disabled={props.disabled}
            checked={props.state.quiet}
            onChange={(e) => set('quiet', e.currentTarget.checked)}
          />
          quiet (-q)
        </label>
        <label>
          <input
            type="checkbox"
            disabled={props.disabled}
            checked={props.state.unbuffered}
            onChange={(e) => set('unbuffered', e.currentTarget.checked)}
          />
          unbuffered
        </label>
        <label>
          <input
            type="checkbox"
            disabled={props.disabled}
            checked={props.state.seq}
            onChange={(e) => set('seq', e.currentTarget.checked)}
          />
          application/json-seq (--seq)
        </label>
      </div>

      <label class="cli-option-extra">
        <span>Extra flags</span>
        <input
          type="text"
          disabled={props.disabled}
          value={props.state.extra}
          placeholder='--arg name "hello world"'
          spellcheck={false}
          autocomplete="off"
          onInput={(e) => set('extra', e.currentTarget.value)}
        />
      </label>
    </div>
  );
};

export default CLIOptionUI;
