# Support module for modules-import.jq (not a standalone test script).
def double: . * 2;
def add_all(arr): reduce arr[] as $x (0; . + $x);
def greet($name): "hello, \($name)!";
