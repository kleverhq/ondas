FROM rust:1.98.0-bookworm

ARG JUST_VERSION=1.58.0
ARG USERNAME=vscode
ARG USER_UID=1000
ARG USER_GID=1000

RUN rustup component add clippy rustfmt \
    && rustup toolchain install 1.85.0 --profile minimal \
    && cargo install just --locked --version "${JUST_VERSION}" \
    && groupadd --gid "${USER_GID}" "${USERNAME}" \
    && useradd --uid "${USER_UID}" --gid "${USER_GID}" --create-home --shell /bin/bash "${USERNAME}" \
    && mkdir -p "/home/${USERNAME}/.cargo" \
    && chown -R "${USERNAME}:${USERNAME}" "/home/${USERNAME}"

ENV CARGO_HOME=/home/vscode/.cargo
ENV RUSTUP_HOME=/usr/local/rustup
ENV PATH=/usr/local/cargo/bin:/home/vscode/.cargo/bin:${PATH}

USER vscode
