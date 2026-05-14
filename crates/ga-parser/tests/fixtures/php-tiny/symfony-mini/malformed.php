<?php

namespace App\Broken;

class Partial
{
    public function valid(): int
    {
        return 42;
    }

    // Deliberate syntax error below — tree-sitter must produce partial AST,
    // walker must continue extracting the `valid()` method above.
    public function broken(: void {
        // missing param name + missing close brace

