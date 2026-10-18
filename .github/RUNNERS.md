# Self-hosted runner

The Rust job can run on the maintainer's Linux server instead of GitHub's
runners. It is switched on by the `SELF_HOSTED_LINUX` repository variable, so it
can be turned off at any time and the job goes back to GitHub's runners. The
Docker image and OpenAPI jobs always use GitHub's runners.

Pull requests from forks always use GitHub's runners, so code from outside the
repository never runs on the server.

The server is the one set up for chek-reader; its
[RUNNERS.md](https://github.com/readchek/chek-reader/blob/main/.github/RUNNERS.md#linux-server)
covers installing Docker and the runner. The job needs a runner with the
`self-hosted` and `chek-linux` labels that this repository can use:

1. Register the runner for the `readchek` organization rather than one
   repository, so every repository can use it: run `./config.sh` with
   `--url https://github.com/readchek` and a token from **Organization settings
   → Actions → Runners → New runner**. A runner registered to one repository
   only serves that repository.
2. Under **Organization settings → Actions → Runner groups**, let the group the
   runner is in serve this repository.
3. In this repository, under **Settings → Actions → General → Fork pull request
   workflows from outside collaborators**, choose **Require approval for all
   external contributors**.
4. Once the runner shows as **Idle**, add the repository variable
   `SELF_HOSTED_LINUX` set to `true` under **Settings → Secrets and variables →
   Actions → Variables**. Set it to `false` while the server is down, or queued
   jobs wait for it.

The job runs in the `rust:1.97-slim-trixie` container and keeps the Cargo
registry, target directory and apt downloads in the Docker volumes
`chek-cargo-registry`, `chek-cargo-target`, `chek-apt-cache` and
`chek-apt-lists`. If a build behaves oddly, remove them with
`docker volume rm chek-cargo-registry chek-cargo-target chek-apt-cache chek-apt-lists`.
