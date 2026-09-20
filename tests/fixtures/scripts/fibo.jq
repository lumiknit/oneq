# Calculate fibonacci
# For each number, calculate the fibonacci number.

def _fibo(n; v):
  v as [$a, $b] |
  if n <= 0 then
	$a
  else
	_fibo(n - 1; [$b, $a + $b])
  end;

def fibo(n): # It requires an arg.
if n < 0 then
  error("Input must be a non-negative integer")
else
  _fibo(n; [0, 1])
  end;

def fibo:
  # This just handle with the input from context.
  tonumber as $n | if $n < 100 then fibo($n), ($n | sin), ($n | log) end;

fibo
