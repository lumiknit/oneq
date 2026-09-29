import {
  batch,
  createSignal,
  For,
  onCleanup,
  onMount,
  type Component,
} from 'solid-js';
import CLIOption from './CLIOption';
import CodeEdit from './CodeEdit';
import { getOutputExtension, parseAnsiSegments, parseOptions } from './options';

type WorkerMessage =
  | { type: 'output'; channel: 1 | 2; bytes: Uint8Array }
  | { type: 'done'; code: number }
  | { type: 'build-configuration'; value: string }
  | { type: 'failure'; message: string }
  | { type: 'cancelled' };

const defaultOption = '-C';
const defaultFilter = '.[] | {name, total: (.values | add)}';
const defaultStdin =
  '[\n  {"name": "alpha", "values": [1, 2, 3]},\n  {"name": "beta", "values": [4, 5]}\n]';

type OutputPart = { channel: 1 | 2; text: string };

type HistoryEntry = {
  id: number;
  runAt: Date;
  options: string;
  filter: string;
  stdin: string;
  stdout: string;
  stderr: string;
  status: string;
  extension: string;
};

// `resume()` states from src/lib_wasm.rs; these are not CLI exit codes.
const runStates: Record<number, string> = {
  2: 'Done',
  3: 'Error',
  4: 'Halted',
};
const runDone = 2;

const outputExtensionOf = (options: string) => {
  let format = 'json';
  const args = parseOptions(options);
  args.forEach((arg, index) => {
    if (arg === '-T' || arg === '--to') format = args[index + 1] ?? format;
    else if (arg.startsWith('--to=')) format = arg.slice(5);
    else if (/^-T./.test(arg)) format = arg.slice(2);
  });
  return getOutputExtension(format);
};

const maxHistory = 50;
// Rendering huge outputs as spans freezes the page; show more on demand.
const outputPreviewLimit = 16 * 1024;

const limitParts = (parts: OutputPart[], limit: number) => {
  const visible: OutputPart[] = [];
  let remaining = limit;
  for (const part of parts) {
    if (remaining <= 0) break;
    let text = part.text;
    if (text.length > remaining) {
      text = text.slice(0, remaining);
      // Do not leave half of an ANSI escape sequence at the cut.
      const escape = text.lastIndexOf('\x1b');
      if (escape >= 0 && !/^\x1b\[[0-?]*[ -/]*[@-~]/.test(text.slice(escape)))
        text = text.slice(0, escape);
    }
    visible.push(text === part.text ? part : { ...part, text });
    remaining -= part.text.length;
  }
  return visible;
};

const stripAnsi = (text: string) =>
  text.replace(/\x1b\[[0-?]*[ -/]*[@-~]/g, '');

const trimPreview = (text: string, max = 80) => {
  const line = text.replace(/\s+/g, ' ').trim();
  return line.length > max ? `${line.slice(0, max)}…` : line;
};

/** Reads state from the `#/?...` fragment (falls back to legacy `?...`). */
const readURLParams = () => {
  if (location.hash.startsWith('#/'))
    return new URL(location.hash.slice(1), location.origin).searchParams;
  return new URLSearchParams(location.search);
};

const renderOutput = (parts: OutputPart[]) => {
  return parts.flatMap((part, partIndex) => {
    const segments = parseAnsiSegments(part.text);
    const nodes = segments.map((seg) => {
      const className =
        [
          part.channel === 2 ? 'stderr-text' : seg.color,
          seg.bold && 'ansi-bold',
          seg.italic && 'ansi-italic',
        ]
          .filter(Boolean)
          .join(' ') || undefined;

      return <span class={className}>{seg.text}</span>;
    });

    if (!nodes.length) {
      nodes.push(
        <span class={part.channel === 2 ? 'stderr-text' : undefined}>
          {part.text}
        </span>,
      );
    }
    return [partIndex ? <span class="output-boundary" /> : null, ...nodes];
  });
};

const App: Component = () => {
  const [options, setOptions] = createSignal(defaultOption);
  const [filter, setFilter] = createSignal(defaultFilter);
  const [stdin, setStdin] = createSignal(defaultStdin);

  onMount(() => {
    const params = readURLParams();

    const pOptions = params.get('options');
    if (pOptions) setOptions(pOptions);

    const pFilter = params.get('filter');
    if (pFilter) setFilter(pFilter);

    const pStdin = params.get('stdin');
    if (pStdin) setStdin(pStdin);
  });

  const setURL = () => {
    const params = new URLSearchParams();

    params.set('options', options());
    params.set('filter', filter());
    const s = stdin();
    if (s.length <= 6000) {
      params.set('stdin', stdin());
    } else {
      params.set('stdin', '');
    }

    history.replaceState(null, '', `${location.pathname}#/?${params}`);
  };

  const [output, setOutput] = createSignal<OutputPart[]>([]);
  const [showAllOutput, setShowAllOutput] = createSignal(false);
  const [running, setRunning] = createSignal(false);
  const [formatting, setFormatting] = createSignal(false);
  const [formatStyle, setFormatStyle] = createSignal<
    'pretty' | 'oneline' | 'compact'
  >('pretty');
  const [buildConfiguration, setBuildConfiguration] = createSignal('Loading…');
  const [status, setStatus] = createSignal('Ready');
  const [autoRun, setAutoRun] = createSignal(true);
  const [runHistory, setRunHistory] = createSignal<HistoryEntry[]>([]);
  let historyId = 0;
  let outputExtension = 'json';
  let currentRun:
    Omit<HistoryEntry, 'stdout' | 'stderr' | 'status'> | undefined;
  // One Worker is reused across runs; it is only recreated after a failure.
  let worker: Worker | undefined;
  let onRunMessage: ((data: WorkerMessage) => void) | undefined;
  let pendingCancels = 0;
  let autoRunTimer: number | undefined;
  let urlTimer: number | undefined;

  const dropWorker = () => {
    worker?.terminate();
    worker = undefined;
    pendingCancels = 0;
  };

  const ensureWorker = () => {
    if (worker) return worker;
    const current = new Worker(
      new URL('../../web/worker.js', import.meta.url),
      { type: 'module' },
    );
    worker = current;
    current.onmessage = ({ data }: MessageEvent<WorkerMessage>) => {
      if (worker !== current) return;
      if (data.type === 'build-configuration') {
        setBuildConfiguration(data.value);
      } else if (data.type === 'cancelled') {
        pendingCancels = Math.max(0, pendingCancels - 1);
      } else if (pendingCancels === 0) {
        // Messages before a pending "cancelled" belong to a cancelled run.
        onRunMessage?.(data);
      }
    };
    current.onerror = (event) => {
      if (worker !== current) return;
      setBuildConfiguration((previous) =>
        previous === 'Loading…' ? 'Unavailable' : previous,
      );
      const handler = onRunMessage;
      dropWorker();
      handler?.({
        type: 'failure',
        message: event.message || 'Worker failed to load or execute.',
      });
    };
    current.onmessageerror = () => {
      if (worker !== current) return;
      const handler = onRunMessage;
      dropWorker();
      handler?.({
        type: 'failure',
        message: 'Could not read the Worker response.',
      });
    };
    return current;
  };
  let frame: number | undefined;
  let pending: OutputPart[] = [];
  let decoders = { 1: new TextDecoder(), 2: new TextDecoder() };

  const flush = () => {
    if (frame !== undefined) cancelAnimationFrame(frame);
    frame = undefined;
    const parts = pending;
    pending = [];
    if (parts.length) setOutput((previous) => [...previous, ...parts]);
  };

  const append = (channel: 1 | 2, text: string) => {
    pending.push({ channel, text });
    frame ??= requestAnimationFrame(flush);
  };

  const finish = (message: string) => {
    onRunMessage = undefined;
    const out = decoders[1].decode();
    const err = decoders[2].decode();
    if (out) pending.push({ channel: 1, text: out });
    if (err) pending.push({ channel: 2, text: err });
    flush();
    if (currentRun) {
      const parts = output();
      const text = (channel: 1 | 2) =>
        stripAnsi(
          parts
            .filter((p) => p.channel === channel)
            .map((p) => p.text)
            .join(''),
        );
      const entry = {
        ...currentRun,
        stdout: text(1),
        stderr: text(2),
        status: message,
      };
      currentRun = undefined;
      setRunHistory((previous) => [entry, ...previous].slice(0, maxHistory));
    }
    batch(() => {
      setRunning(false);
      setFormatting(false);
      setStatus(message);
    });
  };

  const start = (action: 'run' | 'format' = 'run') => {
    if (running()) return;
    flush();
    decoders = { 1: new TextDecoder(), 2: new TextDecoder() };
    currentRun =
      action === 'run'
        ? {
            id: ++historyId,
            runAt: new Date(),
            options: options(),
            filter: filter(),
            stdin: stdin(),
            extension: outputExtensionOf(options()),
          }
        : undefined;
    // Format output is a jq program.
    outputExtension = currentRun?.extension ?? 'jq';
    batch(() => {
      setOutput([]);
      setShowAllOutput(false);
      setStatus(action === 'format' ? 'Formatting…' : 'Running…');
      setRunning(true);
      setFormatting(action === 'format');
    });

    try {
      const args =
        action === 'format'
          ? [
              '--fmt',
              '-C',
              ...(formatStyle() === 'oneline'
                ? ['--inline-output']
                : formatStyle() === 'compact'
                  ? ['-c']
                  : []),
            ]
          : parseOptions(options());
      // Formatting uses stdin as its source.
      if (!args.includes('--fmt')) args.push('--', filter());
      const encoder = new TextEncoder();
      const header = encoder.encode(JSON.stringify(args));
      const input = encoder.encode(action === 'format' ? filter() : stdin());
      const packet = new Uint8Array(header.length + 1 + input.length);
      packet.set(header);
      packet.set(input, header.length + 1);

      onRunMessage = (data) => {
        if (data.type === 'output') {
          append(
            data.channel,
            decoders[data.channel].decode(data.bytes, { stream: true }),
          );
        } else if (data.type === 'done') {
          finish(
            action === 'format' && data.code === runDone
              ? 'Formatted'
              : (runStates[data.code] ?? 'Error'),
          );
          // Keep the original filter on errors or cancellation.
          if (action === 'format' && data.code === runDone)
            setFilter(
              output()
                .filter((p) => p.channel === 1)
                .map((p) => p.text)
                .join('')
                .replace(/\x1b\[[0-9;]*m/g, ''),
            );
        } else if (data.type === 'failure') {
          // A trap may leave the WASM instance unusable; start fresh next time.
          dropWorker();
          append(2, `${data.message}\n`);
          finish('Failed');
        }
      };
      ensureWorker().postMessage({ type: 'start', packet }, [packet.buffer]);
    } catch (error) {
      append(2, `${error instanceof Error ? error.message : String(error)}\n`);
      finish('Failed');
    }
  };

  const cancelRun = () => {
    if (!running()) return;
    if (worker) {
      pendingCancels++;
      worker.postMessage({ type: 'cancel' });
    }
    finish('Cancelled');
  };

  const scheduleAutoRun = () => {
    if (urlTimer !== undefined) window.clearTimeout(urlTimer);
    urlTimer = window.setTimeout(() => {
      urlTimer = undefined;
      setURL();
    }, 500);
    if (autoRunTimer !== undefined) window.clearTimeout(autoRunTimer);
    if (!autoRun()) return;
    if (running()) cancelRun();
    autoRunTimer = window.setTimeout(() => {
      autoRunTimer = undefined;
      if (!running() && autoRun()) start();
    }, 500);
  };

  const outputLength = () =>
    output().reduce((sum, part) => sum + part.text.length, 0);
  const outputTruncated = () =>
    !showAllOutput() && outputLength() > outputPreviewLimit;

  const outputText = () =>
    stripAnsi(
      output()
        .map((part) => part.text)
        .join(''),
    );

  const stdoutText = () =>
    stripAnsi(
      output()
        .filter((part) => part.channel === 1)
        .map((part) => part.text)
        .join(''),
    );

  const runOutputAction = (action: string) => {
    if (action === 'copy') copyOutput();
    else if (action === 'download') downloadOutput();
    else if (action === 'stdin') {
      setStdin(stdoutText());
      scheduleAutoRun();
    } else if (action === 'jq') {
      setFilter(stdoutText());
      scheduleAutoRun();
    }
  };

  const restoreHistory = (entry: HistoryEntry) => {
    if (running()) cancelRun();
    if (autoRunTimer !== undefined) {
      window.clearTimeout(autoRunTimer);
      autoRunTimer = undefined;
    }
    batch(() => {
      setOptions(entry.options);
      setFilter(entry.filter);
      setStdin(entry.stdin);
      setStatus('Restored from history');
    });
    setURL();
    window.scrollTo({ top: 0, behavior: 'smooth' });
  };

  const copyOutput = async () => {
    await navigator.clipboard.writeText(outputText());
    setStatus('Copied');
  };

  const downloadOutput = () => {
    const extension = outputExtension;
    const url = URL.createObjectURL(
      new Blob([outputText()], { type: 'text/plain;charset=utf-8' }),
    );
    const anchor = document.createElement('a');
    anchor.href = url;
    anchor.download = `1q-output.${extension}`;
    anchor.click();
    URL.revokeObjectURL(url);
  };

  onCleanup(() => {
    dropWorker();
    if (autoRunTimer !== undefined) window.clearTimeout(autoRunTimer);
    if (urlTimer !== undefined) window.clearTimeout(urlTimer);
    if (frame !== undefined) cancelAnimationFrame(frame);
  });

  return (
    <main class="container">
      <header class="page-header">
        <div>
          <h1>1q playground</h1>
          <p>Run jq filters in your browser.</p>
          <nav class="repository-links" aria-label="Source repositories">
            <span>For CLI usage and source code, visit:</span>
            <a
              href="https://github.com/lumiknit/1q"
              target="_blank"
              rel="noreferrer"
            >
              GitHub
            </a>
            <a
              href="https://codeberg.org/lumiknit/1q"
              target="_blank"
              rel="noreferrer"
            >
              Codeberg
            </a>
          </nav>
        </div>
      </header>

      <details
        class="build-configuration"
        onToggle={(event) => {
          if (event.currentTarget.open) ensureWorker();
        }}
      >
        <summary>WASM build configuration</summary>
        <pre>{buildConfiguration()}</pre>
      </details>

      <form
        onSubmit={(event) => {
          event.preventDefault();
          start();
        }}
        onKeyDown={(event) => {
          if ((event.ctrlKey || event.metaKey) && event.key === 'Enter') {
            event.preventDefault();
            start();
          }
        }}
      >
        <div class="options-field">
          <span>Options</span>
          <CLIOption
            options={options}
            setOptions={(value) => {
              setOptions(value);
              scheduleAutoRun();
            }}
            disabled={running()}
          />
        </div>
        <div class="editors">
          <label class="panel stdin-panel">
            <div class="editor-header">
              <span>stdin</span>
              <span class="file-action">
                <input
                  type="file"
                  hidden
                  accept=".json,.jsonl,.txt,text/*,application/json"
                  onChange={(event) => {
                    const file = event.currentTarget.files?.[0];
                    if (file)
                      file.text().then((value) => {
                        setStdin(value);
                        scheduleAutoRun();
                      });
                  }}
                />
                <button
                  type="button"
                  class="secondary"
                  disabled={running()}
                  onClick={(event) =>
                    (
                      event.currentTarget
                        .previousElementSibling as HTMLInputElement
                    ).click()
                  }
                >
                  Load from file
                </button>
              </span>
            </div>
            <CodeEdit
              value={stdin()}
              onInput={(value) => {
                setStdin(value);
                scheduleAutoRun();
              }}
              aria-label="stdin"
            />
          </label>
          <div class="panel">
            <div class="editor-header">
              <label for="jq-input">jq</label>
              <div class="format-controls">
                <select
                  class="format-actions"
                  aria-label="Format jq filter"
                  disabled={running()}
                  onChange={(event) => {
                    const style = event.currentTarget.value;
                    // Act like a dropdown menu: always snap back to "Format".
                    event.currentTarget.value = '';
                    if (!style) return;
                    setFormatStyle(style as 'pretty' | 'oneline' | 'compact');
                    start('format');
                  }}
                >
                  <option value="" selected>
                    Format
                  </option>
                  <option value="pretty">Pretty</option>
                  <option value="oneline">Oneline</option>
                  <option value="compact">Compact</option>
                </select>
              </div>
            </div>
            <CodeEdit
              id="jq-input"
              value={filter()}
              readOnly={formatting()}
              onInput={(value) => {
                setFilter(value);
                scheduleAutoRun();
              }}
              aria-label="jq filter"
            />
          </div>
        </div>
        <div class="actions">
          {running() ? (
            <button type="button" class="secondary" onClick={cancelRun}>
              Cancel
            </button>
          ) : (
            <button type="submit">Run</button>
          )}
          <label>
            <input
              type="checkbox"
              checked={autoRun()}
              onChange={(event) => {
                setAutoRun(event.currentTarget.checked);
                if (
                  !event.currentTarget.checked &&
                  autoRunTimer !== undefined
                ) {
                  window.clearTimeout(autoRunTimer);
                  autoRunTimer = undefined;
                }
              }}
            />{' '}
            Auto-run
          </label>
          <span class="status" classList={{ running: running() }} role="status">
            {status()}
          </span>
          <span>Ctrl / ⌘ + Enter to run</span>
        </div>
      </form>

      <section class="outputs" aria-label="Output">
        <div class="panel output-panel">
          <h2>
            <span>stdout + stderr</span>
            <select
              class="output-actions"
              aria-label="Output actions"
              disabled={running() || !output().length}
              onChange={(event) => {
                const action = event.currentTarget.value;
                // Act like a dropdown menu: always snap back to "Actions".
                event.currentTarget.value = '';
                runOutputAction(action);
              }}
            >
              <option value="" selected>
                Actions
              </option>
              <option value="copy">Copy to Clipboard</option>
              <option value="download">Download</option>
              <option value="stdin">Use as STDIN</option>
              <option value="jq">Use as JQ</option>
            </select>
          </h2>
          <div class="output" tabindex="0">
            {renderOutput(
              outputTruncated()
                ? limitParts(output(), outputPreviewLimit)
                : output(),
            )}
            {outputTruncated() ? (
              <button
                type="button"
                class="secondary output-more"
                onClick={() => setShowAllOutput(true)}
              >
                Show all ({Math.ceil(outputLength() / 1024)} KB)
              </button>
            ) : null}
          </div>
        </div>
      </section>

      <section class="run-history" aria-label="Run history">
        <h2>
          <span>History ({runHistory().length})</span>
          <button
            type="button"
            class="secondary"
            disabled={!runHistory().length}
            onClick={() => setRunHistory([])}
          >
            Clear
          </button>
        </h2>
        {runHistory().length ? (
          <ul>
            <For each={runHistory()}>
              {(entry, index) => (
                <li>
                  <button
                    type="button"
                    class="history-entry"
                    title="Restore this run into the form"
                    onClick={() => restoreHistory(entry)}
                  >
                    <div class="history-meta">
                      <span class="history-index">
                        #{runHistory().length - index()}
                      </span>
                      <time datetime={entry.runAt.toISOString()}>
                        {entry.runAt.toLocaleString()}
                      </time>
                      <span class="history-status">{entry.status}</span>
                      <span>.{entry.extension}</span>
                      <code>
                        {trimPreview(entry.options, 40) || '(no options)'}
                      </code>
                    </div>
                    <div class="history-io">
                      <span class="history-label">jq</span>
                      <code>{trimPreview(entry.filter)}</code>
                      <span class="history-label">stdin</span>
                      <code>{trimPreview(entry.stdin) || '(empty)'}</code>
                      <span class="history-label">out</span>
                      <code>{trimPreview(entry.stdout) || '(empty)'}</code>
                      {entry.stderr ? (
                        <>
                          <span class="history-label">err</span>
                          <code class="stderr-text">
                            {trimPreview(entry.stderr)}
                          </code>
                        </>
                      ) : null}
                    </div>
                  </button>
                </li>
              )}
            </For>
          </ul>
        ) : (
          <p class="history-empty">No runs yet.</p>
        )}
      </section>
    </main>
  );
};

export default App;
