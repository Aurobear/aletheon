#compdef aletheon

_aletheon_cli_completion() {
  local -a commands
  commands=(
    'core:start the machine inference core'
    'daemon:start the user daemon'
    'exec:run a non-interactive task'
    'version:print version information'
    'restore-terminal:restore terminal modes'
    'config:inspect effective configuration'
    'doctor:run diagnostics'
    'extension:manage extensions'
    'memory-agent:run memory maintenance'
    'memory:use the governed Memory Gateway'
  )
  if (( CURRENT == 2 )); then
    _describe 'command' commands
    return
  fi
  case ${words[2]:-} in
    config) _values 'action' effective layers ;;
    extension) _values 'action' inspect validate install list show enable disable upgrade rollback remove purge doctor import-legacy ;;
    memory-agent) _values 'action' serve run ;;
    memory)
      if (( CURRENT == 3 )); then
        _values 'action' observe recall receipt workspace
      elif [[ ${words[3]:-} == workspace && CURRENT == 4 ]]; then
        _values 'action' preview-bind bind unbind
      fi
      ;;
    exec) _arguments '--prompt[task prompt]:' '--model[model route]:' '--max-turns[maximum turns]:' '--sandbox[sandbox preference]:(auto require forbid)' '--config[config file]:_files' '--output[output format]:(text json)' ;;
  esac
}

compdef _aletheon_cli_completion aletheon
