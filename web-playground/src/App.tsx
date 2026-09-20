import {
  batch,
  createSignal,
  onCleanup,
  onMount,
  type Component,
} from 'solid-js';
import CLIOption from './CLIOption';
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
    const params = new URLSearchParams(location.search);

    const pOptions = params.get('options');
    if (pOptions) setOptions(pOptions);

    const pFilter = params.get('filter');
    if (pFilter) setFilter(pFilter);

    const pStdin = params.get('stdin');
    if (pStdin) setStdin(pStdin);
  });

  const setURL = () => {
    const params = new URLSearchParams(location.search);

    params.set('options', options());
    params.set('filter', filter());
    const s = stdin();
    if (s.length <= 6000) {
      params.set('stdin', stdin());
    } else {
      params.set('stdin', '');
    }

    history.replaceState(
      null,
      '',
      `${location.pathname}?${params}${location.hash}`,
    );
  };

  const [output, setOutput] = createSignal<OutputPart[]>([]);
  const [running, setRunning] = createSignal(false);
  const [formatting, setFormatting] = createSignal(false);
  const [formatStyle, setFormatStyle] = createSignal<
    'pretty' | 'oneline' | 'compact'
  >('pretty');
  const [buildConfiguration, setBuildConfiguration] = createSignal('Loading…');
  const [status, setStatus] = createSignal('Ready');
  const [autoRun, setAutoRun] = createSignal(true);
  let worker: Worker | undefined;
  let autoRunTimer: number | undefined;
  let urlTimer: number | undefined;
  const buildWorker = new Worker(
    new URL('../../web/worker.js', import.meta.url),
    {
      type: 'module',
    },
  );
  buildWorker.onmessage = ({ data }: MessageEvent<WorkerMessage>) => {
    if (data.type === 'build-configuration') setBuildConfiguration(data.value);
  };
  buildWorker.onerror = () => setBuildConfiguration('Unavailable');
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
    worker?.terminate();
    worker = undefined;
    const out = decoders[1].decode();
    const err = decoders[2].decode();
    if (out) pending.push({ channel: 1, text: out });
    if (err) pending.push({ channel: 2, text: err });
    flush();
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
    batch(() => {
      setOutput([]);
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

      const current = new Worker(
        new URL('../../web/worker.js', import.meta.url),
        {
          type: 'module',
        },
      );
      worker = current;
      current.onmessage = ({ data }: MessageEvent<WorkerMessage>) => {
        if (worker !== current) return;
        if (data.type === 'output') {
          console.log(
            '[1q playground output]',
            data.channel === 1 ? 'stdout' : 'stderr',
            data.bytes,
          );
          append(
            data.channel,
            decoders[data.channel].decode(data.bytes, { stream: true }),
          );
        } else if (data.type === 'build-configuration') {
          setBuildConfiguration(data.value);
        } else if (data.type === 'done') {
          finish(
            action === 'format' && data.code === 0
              ? 'Formatted'
              : `Exit ${data.code}`,
          );
          // Keep the original filter on errors or cancellation.
          if (action === 'format' && data.code === 0)
            setFilter(
              output()
                .filter((p) => p.channel === 1)
                .map((p) => p.text)
                .join('')
                .replace(/\x1b\[[0-9;]*m/g, ''),
            );
        } else if (data.type === 'failure') {
          append(2, `${data.message}\n`);
          finish('Failed');
        }
      };
      current.onerror = (event) => {
        if (worker !== current) return;
        append(2, `${event.message || 'Worker failed to load or execute.'}\n`);
        finish('Failed');
      };
      current.onmessageerror = () => {
        if (worker !== current) return;
        append(2, 'Could not read the Worker response.\n');
        finish('Failed');
      };
      current.postMessage({ type: 'start', packet }, [packet.buffer]);
    } catch (error) {
      append(2, `${error instanceof Error ? error.message : String(error)}\n`);
      finish('Failed');
    }
  };

  const cancelRun = () => {
    if (!running()) return;
    worker?.postMessage({ type: 'cancel' });
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

  const outputText = () => {
    return output()
      .map((part) => part.text)
      .join('')
      .replace(/\x1b\[[0-?]*[ -/]*[@-~]/g, '');
  };

  const copyOutput = async () => {
    await navigator.clipboard.writeText(outputText());
    setStatus('Copied');
  };

  const downloadOutput = () => {
    const args = parseOptions(options());
    const formatIndex = args.findIndex((arg) => arg === '-T' || arg === '--to');
    const format = formatIndex >= 0 ? args[formatIndex + 1] : 'json';
    const extension = getOutputExtension(format);
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
    buildWorker.terminate();
    worker?.terminate();
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

      <details class="build-configuration">
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
            <textarea
              value={stdin()}
              onInput={(event) => {
                setStdin(event.currentTarget.value);
                scheduleAutoRun();
              }}
              spellcheck={false}
              aria-label="stdin"
            />
          </label>
          <div class="panel">
            <div class="editor-header">
              <label for="jq-input">jq</label>
              <div class="format-controls">
                <select
                  aria-label="jq formatting style"
                  value={formatStyle()}
                  disabled={running()}
                  onChange={(event) =>
                    setFormatStyle(
                      event.currentTarget.value as
                        'pretty' | 'oneline' | 'compact',
                    )
                  }
                >
                  <option value="pretty">Pretty</option>
                  <option value="oneline">Oneline</option>
                  <option value="compact">Compact</option>
                </select>
                <button
                  type="button"
                  disabled={running()}
                  onClick={() => start('format')}
                >
                  Format
                </button>
              </div>
            </div>
            <textarea
              id="jq-input"
              value={filter()}
              readOnly={formatting()}
              onInput={(event) => {
                setFilter(event.currentTarget.value);
                scheduleAutoRun();
              }}
              spellcheck={false}
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
            <span class="output-actions">
              <button
                type="button"
                class="secondary"
                disabled={!output().length}
                onClick={copyOutput}
              >
                Copy
              </button>
              <button
                type="button"
                class="secondary"
                disabled={!output().length}
                onClick={downloadOutput}
              >
                Download
              </button>
            </span>
          </h2>
          <div class="output" tabindex="0">
            {renderOutput(output())}
          </div>
        </div>
      </section>
    </main>
  );
};

export default App;
