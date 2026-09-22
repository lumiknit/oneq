def f: if . == 0 then (1, 2) else try (.-1 | f) catch . end; [f]
