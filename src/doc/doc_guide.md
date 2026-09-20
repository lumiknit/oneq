# DOC Guide

## Schema

```typescript
export type Doc = {
  // Referenced jq version, current is 1.8
	version: string;
	syntax: Syntax[];
	operators: Operator[];
	builtins: Builtin[];
}

// Description for syntax, for example: comment, pipe, foreach, module, etc.
type Syntax = {
  // Syntax form. Human-readable.
	form: string;
	description: string;
}

type Operator = {
  name: string;
  kind: 'binary' | 'prefix' | 'suffix';
  precedence: number;
  description: string;
}

// Builtin functions or filter references
type Builtin = {
  // Name is function name in most case
  name: string;

	// Sig is signatures.
	// For function, it may be '()' (no args), '(regex; flags)' (arguments)
	// Each entry contains single argument forms.
	// Some special case, such as foreach, may contains syntax form.
  sig: string[];

  // Type of input, as typescript notation
  input: string;

  // Type of output, as typescript notation
  output: string;

  // Detail description
  description: string;

  // Category
  category: string[];
}
```
