# Bash completion for the installed Aletheon CLI.

_aletheon_cli_completion() {
    local cur prev command sub
    COMPREPLY=()
    cur=${COMP_WORDS[COMP_CWORD]}
    prev=${COMP_WORDS[COMP_CWORD-1]:-}
    command=${COMP_WORDS[1]:-}
    sub=${COMP_WORDS[2]:-}

    case "$prev" in
        -C|--working-dir|--add-dir|--config|-c|--env|--project-dir|-d)
            COMPREPLY=($(compgen -d -- "$cur")); return ;;
        path)
            COMPREPLY=($(compgen -f -- "$cur")); return ;;
        -P|--permission-mode)
            COMPREPLY=($(compgen -W "safe dev full" -- "$cur")); return ;;
        --output)
            COMPREPLY=($(compgen -W "text json" -- "$cur")); return ;;
        --sandbox)
            COMPREPLY=($(compgen -W "auto require forbid" -- "$cur")); return ;;
    esac

    if ((COMP_CWORD == 1)); then
        COMPREPLY=($(compgen -W "core daemon exec version restore-terminal config doctor extension memory-agent memory --help --version --full --permission-mode --message --socket --task-kind -C --add-dir" -- "$cur"))
        return
    fi

    case "$command" in
        config)
            ((COMP_CWORD == 2)) && COMPREPLY=($(compgen -W "effective layers" -- "$cur"))
            ;;
        extension)
            ((COMP_CWORD == 2)) && COMPREPLY=($(compgen -W "inspect validate install list show enable disable upgrade rollback remove purge doctor" -- "$cur"))
            ;;
        memory-agent)
            ((COMP_CWORD == 2)) && COMPREPLY=($(compgen -W "serve run" -- "$cur"))
            ;;
        memory)
            if ((COMP_CWORD == 2)); then
                COMPREPLY=($(compgen -W "observe recall receipt workspace" -- "$cur"))
            elif [[ $sub == workspace && $COMP_CWORD -eq 3 ]]; then
                COMPREPLY=($(compgen -W "preview-bind bind unbind" -- "$cur"))
            fi
            ;;
        exec)
            COMPREPLY=($(compgen -W "--prompt --model --max-turns --sandbox --config --output" -- "$cur"))
            ;;
        daemon)
            COMPREPLY=($(compgen -W "--config --env --socket --container --image --enable-evolution --execd" -- "$cur"))
            ;;
        doctor)
            COMPREPLY=($(compgen -W "--json --config --project-dir" -- "$cur"))
            ;;
    esac
}

complete -F _aletheon_cli_completion aletheon
