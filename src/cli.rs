//! The text behind `--help`. Kept to 80 columns; a test holds it to the flags
//! that `web::Options::parse` really accepts.
pub const HELP: &str = "\
Vaayu - a modal code editor with Vim-style editing, language servers and project
tools, in your terminal or a browser tab.

USAGE
  vaayu [FILE]                    Edit in this terminal
  vaayu --web [OPTIONS] [FILE]    Edit in a browser tab
  vaayu --help | --version

  `vy` is the same program under a shorter name.

ARGUMENTS
  FILE    The file to open. A path that doesn't exist yet opens as a new file
          and is created when you save. Only one file is opened, and it can't
          be a directory.

  The current directory is the project, wherever FILE is, so start Vaayu from
  the project you want to work in. Without FILE you get an empty buffer.

OPTIONS
  -h, --help       Print this help and exit
  -V, --version    Print the version and exit

BROWSER OPTIONS
  --web            Run the editor as a local server and open it in your default
                   browser. It is the same editor, with the same config and
                   features. Ctrl-C here stops it, as does :q in the tab.
  --host ADDR      IP address to listen on (default: 127.0.0.1). 0.0.0.0 accepts
                   connections from other machines. WARNING: the connection is
                   not encrypted, so anyone who can watch the network can take
                   the link, read your files and run commands as you. Use it
                   only on a network you trust, and prefer an SSH tunnel.
  --port N         Port to listen on (default: first free from 7420). A busy
                   port is an error.
  --no-open        Print the link without starting a browser.

  --host, --port and --no-open need --web; without it they are read as FILE.

  Each project has one browser session. Running --web again reuses it, opening
  FILE there; to change --host, quit the session first. Reloading or closing
  the tab keeps the session, including unsaved text. With no tab attached and
  nothing to lose, it stops by itself after 30 minutes.

  Browsers keep Ctrl-W, Ctrl-T and Ctrl-N for themselves: press Alt-W, Alt-T and
  Alt-N instead, or F11 for fullscreen, where Chromium passes the real keys.

FIRST STEPS IN THE EDITOR
  i, then Esc      Start and stop inserting text
  Ctrl-S or :w     Save
  :q               Quit; this stops if there is unsaved work
  :qa!             Quit and discard unsaved work
  :help            Search the built-in help and keymaps
  :checkhealth     Check language servers and external tools
  :web [FILE]      Open the browser editor from the terminal editor

  The leader key is , by default: ,w saves, for example.

FILES
  ~/.config/vaayu/config.toml      Settings and keymaps; all fields are optional
  ~/.config/vaayu/dictionary.txt   Your spelling dictionary
  ~/.local/share/vaayu/tasks/      Personal task documents
  .vaayu/ in the project           Comments, undo history, recovery snapshots,
                                   session state and the private browser link

  Those are the Linux locations; other systems use their own config and data
  directories.

ENVIRONMENT
  XDG_CONFIG_HOME, XDG_DATA_HOME   Move the config and data directories (Linux)
  BROWSER                          Command that opens the --web link
  SHELL                            Shell for embedded terminals
  VAAYU_PROFILE                    File to write per-frame timings to

OPTIONAL TOOLS
  ripgrep          Project file discovery and live grep
  git, gh          Git features; the GitHub workspace (gh must be logged in)
  wl-copy/wl-paste or X11 clipboard tools
                   System clipboard
  language servers Whichever ones you configure

EXAMPLES
  vaayu src/main.rs
      Edit a file in this terminal.
  vaayu --web
      Open the current project in a browser tab.
  vaayu --web --no-open --port 7420
      Serve on a remote machine, then run this on your own and open the link:
      ssh -L 7420:127.0.0.1:7420 HOST
  vaayu --web --host 0.0.0.0 --port 7420
      Open the session to your network. Read the warning above first.

MORE
  GUIDE.md, CONFIGURATION.md and COMMANDS.md in the repository:
  https://github.com/krshrimali/vaayu
";

#[cfg(test)]
mod tests {
    use super::HELP;
    use crate::web::Options;

    #[test]
    fn help_fits_a_standard_terminal() {
        for line in HELP.lines() {
            assert!(line.chars().count() <= 80, "too wide: {line}");
        }
    }

    #[test]
    fn every_documented_web_flag_is_accepted() {
        let args = [
            ("--host", Some("0.0.0.0")),
            ("--port", Some("7420")),
            ("--no-open", None),
        ];
        for (flag, value) in args {
            assert!(HELP.contains(flag), "{flag} is undocumented");
            let mut argv = vec!["--web".to_string(), flag.to_string()];
            argv.extend(value.map(str::to_string));
            assert!(Options::parse(&argv).unwrap().is_some(), "{flag}");
        }
        let documented = HELP
            .split(|c: char| !(c.is_ascii_alphanumeric() || c == '-'))
            .filter(|word| word.starts_with("--"));
        for flag in documented {
            let known = ["--help", "--version", "--web"].contains(&flag)
                || args.iter().any(|(name, _)| *name == flag);
            assert!(known, "{flag} is documented but not handled");
        }
    }
}
