# dotfiles

Basically, here you can find the settings of OSX according to my preferences and automatic installation of applications from different sources. 

If you liked this idea then please don't forget to give me a star. =]

## How to run it?

Yeah, that's really simple, just run the following on your terminal:

```bash
make
```

It will run every job in order: `software_update`, `install_dependencies`, `install_apps`, `configure_apps`, `apply_preferences` and `update_history_commands`.

You can also run any job on its own:

```bash
make software_update
make install_dependencies
make install_apps
make configure_apps
make apply_preferences
make update_history_commands
```

To skip one or more jobs while debugging, list them on the `SKIP_JOBS` environment variable (space or comma separated):

```bash
SKIP_JOBS="software_update" make
SKIP_JOBS="install_apps configure_apps" make
```
