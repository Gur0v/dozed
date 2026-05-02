# dozed
set -l all_events timeout before-sleep after-resume lock unlock
set -l cmd_events before-sleep after-resume lock unlock
set -l time_events timeout

complete -c dozed --arguments "$all_events"
complete -c dozed --condition "__fish_seen_subcommand_from $cmd_events" --require-parameter
complete -c dozed --condition "__fish_seen_subcommand_from $time_events" --exclusive

complete -c dozed -s h --description 'show help'
complete -c dozed -s d --description 'debug'
complete -c dozed -s w --description 'wait for command to finish'
complete -c dozed -l no-config --description 'ignore config files'
complete -c dozed -l dry-run --description 'validate and print events'
complete -c dozed -l validate-config --description 'validate and exit'
complete -c dozed -l print-events --description 'print parsed events'
complete -c dozed -l ignore-fullscreen --description 'do not suppress idle in fullscreen apps'
complete -c dozed -l fullscreen-policy --arguments 'suppress ignore' --description 'fullscreen behavior'
