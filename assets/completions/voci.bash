_voci_complete() {
    local value
    COMPREPLY=()
    while IFS= read -r value || [[ -n $value ]]; do
        if [[ -n $value ]]; then
            printf -v value '%q' "$value"
            COMPREPLY+=("$value")
        fi
    done < <("${COMP_WORDS[0]}" __complete -- "${COMP_WORDS[@]:1:COMP_CWORD}" 2>/dev/null)
}
complete -F _voci_complete voci
