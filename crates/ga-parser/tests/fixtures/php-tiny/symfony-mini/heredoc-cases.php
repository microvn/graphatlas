<?php
// AS-018 adversarial fixture — heredoc / nowdoc forms with PHP-like content.
// extract_calls + extract_references on this file MUST emit ZERO edges for any
// of the unique identifiers below. Tree-sitter-php DOES parse member/function
// calls inside heredoc-body complex interpolation as real call nodes; the PHP
// parser adapter must suppress emission via the heredoc/nowdoc-body ancestor
// check, NOT trust raw walker discovery.

namespace App\Adversarial;

class HeredocCases
{
    // Case 1 — simple heredoc with $var->prop interpolation (parses as
    // member_access_expression, not call — but still inside string body).
    public string $case1 = <<<PHP
  before
  $payload->absorbAll
  after
PHP;

    // Case 2 — nowdoc (single-quoted heredoc) — fully literal, no PHP parsing.
    public string $case2 = <<<'NOWDOC'
  $nowdocCall->shouldNotResolve();
  Nowdoc::staticPoison();
  new NowdocEvil();
NOWDOC;

    // Case 3 — heredoc with complex interpolation `{$var->m()}` — tree-sitter
    // PARSES this as member_call_expression. Phantom-edge risk is REAL here.
    public string $case3 = <<<EOT
  prefix
  {$heredocComplexReceiver->heredocPhantomMethod()}
  suffix
EOT;

    // Case 4 — heredoc with `${func(...)}` dynamic-variable form — tree-sitter
    // parses the inner as function_call_expression.
    public string $case4 = <<<EOT
  intro
  ${heredocPhantomFunction("evil")}
  outro
EOT;

    // Case 5 — heredoc with scoped_call shape `{Class::method()}` — also
    // parses as scoped_call_expression inside the interpolation.
    public string $case5 = <<<EOT
  alpha
  {HeredocPhantomClass::heredocPhantomStatic()}
  omega
EOT;

    // Case 6 — heredoc with new-expression shape inside interpolation. Even
    // though `new` is unusual in interpolation, the grammar may try parsing.
    public string $case6 = <<<EOT
  open
  {$_ENV['heredocPhantomNew']}
  close
EOT;

    // Case 7 — nested PHP-like content with multi-line statements (raw string
    // content; no parsing trigger). Should produce zero calls.
    public string $case7 = <<<DOC
  This is a docblock example showing usage:
    \$service->callsiteShouldNotResolve();
    HeredocDocstringClass::statSiteShouldNotResolve();
    new HeredocDocstringEvil();
DOC;
}
