# dozed
complete -c dozed -f

complete -c dozed -n 'not __fish_seen_subcommand_from timeout before-sleep after-resume lock unlock resume' -a timeout -d 'Run a command after idle timeout'
complete -c dozed -n 'not __fish_seen_subcommand_from timeout before-sleep after-resume lock unlock resume' -a before-sleep -d 'Run a command before sleep'
complete -c dozed -n 'not __fish_seen_subcommand_from timeout before-sleep after-resume lock unlock resume' -a after-resume -d 'Run a command after resume'
complete -c dozed -n 'not __fish_seen_subcommand_from timeout before-sleep after-resume lock unlock resume' -a lock -d 'Run a command on session lock'
complete -c dozed -n 'not __fish_seen_subcommand_from timeout before-sleep after-resume lock unlock resume' -a unlock -d 'Run a command on session unlock'

complete -c dozed -n '__fish_seen_subcommand_from timeout; and not __fish_seen_subcommand_from resume' -a resume -d 'Run a command when activity resumes'
complete -c dozed -n '__fish_seen_subcommand_from before-sleep after-resume lock unlock resume' -a '(__fish_complete_command)'

complete -c dozed -s C -r -F -d 'path to config file'
complete -c dozed -s S -r -d 'Wayland seat to watch'
complete -c dozed -s h -l help -d 'show help'
complete -c dozed -s d -d 'debug'
complete -c dozed -s w -d 'wait for command to finish'
complete -c dozed -l no-config -d 'ignore config files'
complete -c dozed -l dry-run -d 'validate and print events'
complete -c dozed -l validate-config -d 'validate and exit'
complete -c dozed -l print-events -d 'print parsed events'
complete -c dozed -l ignore-fullscreen -d 'do not suppress idle in fullscreen apps'
complete -c dozed -l fullscreen-policy -x -a 'suppress ignore' -d 'fullscreen behavior'
