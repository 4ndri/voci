# Source after any existing external-completer configuration.
let voci_previous_completer = $env.config.completions.external.completer
$env.config.completions.external.enable = true
$env.config.completions.external.completer = {|place, buffer|
    let spans = $place.command
    if (($spans | first | path basename) in ['voci' 'voci.exe']) {
        try {
            # Nushell passes shell syntax in spans, including an unfinished opening quote.
            # Decode string literals without evaluating them before matching saved queries.
            let words = ($spans | skip 1 | each {|word|
                let quote = ($word | str substring 0..0)
                if $quote in ['"' "'"] {
                    try { $word | from nuon } catch { ($word + $quote) | from nuon }
                } else if $quote == '`' {
                    $word | str trim --char '`'
                } else {
                    $word
                }
            })
            let values = (run-external ($spans | first) "--json" "__complete" "--" ...$words | from json)
            $values | each {|value|
                let completed = if $value =~ '^[\p{L}\p{M}\p{N}_./:@%+=,-]+$' {
                    $value
                } else {
                    $value | to nuon
                }
                {value: $completed, description: 'voci'}
            }
        } catch { [] }
    } else if $voci_previous_completer != null {
        # Let Nushell bind the previous completer's named inputs (or legacy spans).
        # Keep this override local so subsequent voci completions still use this adapter.
        do {
            $env.config.completions.external.completer = $voci_previous_completer
            $buffer | commandline complete --detailed
        }
    } else {
        null
    }
}
