#compdef aletheon.sh

_aletheon_operations_completion() {
  local command

  _arguments -C \
    '1:command:(build install deploy configure status health restart logs backup restore upgrade cleanup secrets database verify acceptance test closure completion help)' \
    '*::argument:->arguments'

  command=${words[2]:-}
  case "$command" in
    install)
      _arguments '--no-enable[install without enabling services]'
      ;;
    deploy)
      _arguments \
        '--no-build[skip the release build]' \
        '--no-restart[do not restart services]' \
        '--no-enable[install without enabling services]'
      ;;
    configure)
      _values 'action' show check
      ;;
    logs)
      _values 'service' core user closure
      ;;
    cleanup)
      _values 'target' runtime cargo
      ;;
    secrets)
      _values 'action' init audit
      ;;
    database)
      if ((CURRENT == 3)); then
        _values 'action' check
      else
        _files
      fi
      ;;
    verify)
      _values 'target' systemd network compose migration multi-user
      ;;
    acceptance)
      _values 'target' architecture release
      ;;
    test)
      _values 'suite' unit operations deployment architecture all
      ;;
    closure)
      _values 'action' install run status
      ;;
    completion)
      _values 'shell' bash zsh
      ;;
  esac
}

compdef _aletheon_operations_completion aletheon.sh
