def fibo:
  if . < 2 then
    .
  else
    (. - 1 | fibo) + (. - 2 | fibo)
  end;
fibo
