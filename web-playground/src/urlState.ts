// Playground state is stored in the URL fragment as `#/?z=...`, where `z` is
// deflate-raw compressed JSON encoded with base64url.

declare global {
  interface Uint8Array {
    toBase64(options?: {
      alphabet?: 'base64' | 'base64url';
      omitPadding?: boolean;
    }): string;
  }
  interface Uint8ArrayConstructor {
    fromBase64(
      string: string,
      options?: { alphabet?: 'base64' | 'base64url' },
    ): Uint8Array<ArrayBuffer>;
  }
}

export type URLState = {
  options?: string;
  filter?: string;
  stdin?: string;
};

const pipeBytes = async (
  bytes: Uint8Array<ArrayBuffer>,
  stream: CompressionStream | DecompressionStream,
) => {
  const out = new Blob([bytes]).stream().pipeThrough(stream);
  return new Uint8Array(await new Response(out).arrayBuffer());
};

export const encodeState = async (state: URLState) => {
  const bytes = new TextEncoder().encode(JSON.stringify(state));
  const compressed = await pipeBytes(
    bytes,
    new CompressionStream('deflate-raw'),
  );
  return compressed.toBase64({ alphabet: 'base64url', omitPadding: true });
};

export const decodeState = async (z: string): Promise<URLState> => {
  const bytes = Uint8Array.fromBase64(z, { alphabet: 'base64url' });
  const raw = await pipeBytes(bytes, new DecompressionStream('deflate-raw'));
  const value: unknown = JSON.parse(new TextDecoder().decode(raw));
  if (typeof value !== 'object' || value === null) return {};
  const pick = (key: keyof URLState) => {
    const v = (value as Record<string, unknown>)[key];
    return typeof v === 'string' ? v : undefined;
  };
  return {
    options: pick('options'),
    filter: pick('filter'),
    stdin: pick('stdin'),
  };
};

/** Reads the `z` param from the `#/?z=...` fragment. */
export const readURLState = async (): Promise<URLState> => {
  if (!location.hash.startsWith('#/')) return {};
  const z = new URL(location.hash.slice(1), location.origin).searchParams.get(
    'z',
  );
  if (!z) return {};
  try {
    return await decodeState(z);
  } catch (e) {
    console.warn('Failed to decode URL state', e);
    return {};
  }
};

// Drop stdin from the URL if the encoded state would get too long.
const maxEncodedLength = 8000;

export const writeURLState = async (state: URLState) => {
  let z = await encodeState(state);
  if (z.length > maxEncodedLength)
    z = await encodeState({ ...state, stdin: '' });
  history.replaceState(null, '', `${location.pathname}#/?z=${z}`);
};
