# Bash completion for scripts/aletheon.sh.

_aletheon_operations_completion() {
    local cur command
    cur=${COMP_WORDS[COMP_CWORD]}
    command=${COMP_WORDS[1]:-}

    if ((COMP_CWORD == 1)); then
        COMPREPLY=($(compgen -W \
            "build install deploy configure status health restart logs backup restore upgrade cleanup secrets database verify acceptance test closure completion help" \
            -- "$cur"))
        return
    fi

    case "$command" in
        install)
            COMPREPLY=($(compgen -W "--no-enable" -- "$cur"))
            ;;
        deploy)
            COMPREPLY=($(compgen -W "--no-build --no-restart --no-enable" -- "$cur"))
            ;;
        configure)
            COMPREPLY=($(compgen -W "show check" -- "$cur"))
            ;;
        logs)
            COMPREPLY=($(compgen -W "core user closure" -- "$cur"))
            ;;
        cleanup)
            COMPREPLY=($(compgen -W "runtime cargo" -- "$cur"))
            ;;
        secrets)
            COMPREPLY=($(compgen -W "init audit" -- "$cur"))
            ;;
        database)
            if ((COMP_CWORD == 2)); then
                COMPREPLY=($(compgen -W "check" -- "$cur"))
            else
                COMPREPLY=($(compgen -f -- "$cur"))
            fi
            ;;
        verify)
            COMPREPLY=($(compgen -W "systemd network compose migration multi-user" -- "$cur"))
            ;;
        acceptance)
            COMPREPLY=($(compgen -W "architecture release" -- "$cur"))
            ;;
        test)
            COMPREPLY=($(compgen -W "unit operations deployment architecture all" -- "$cur"))
            ;;
        closure)
            COMPREPLY=($(compgen -W "install run status" -- "$cur"))
            ;;
        completion)
            COMPREPLY=($(compgen -W "bash zsh" -- "$cur"))
            ;;
    esac
}

complete -F _aletheon_operations_completion aletheon.sh
