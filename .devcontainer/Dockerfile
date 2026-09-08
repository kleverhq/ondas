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

ARG PRE_COMMIT_VERSION=4.5.1
RUN apt-get update \
    && apt-get install -y --no-install-recommends python3-venv \
    && rm -rf /var/lib/apt/lists/* \
    && python3 -m venv /opt/ondas-python \
    && /opt/ondas-python/bin/pip install --no-cache-dir "pre-commit==${PRE_COMMIT_VERSION}"

ENV CARGO_HOME=/home/vscode/.cargo
ENV RUSTUP_HOME=/usr/local/rustup
ENV PATH=/opt/ondas-python/bin:/usr/local/cargo/bin:/home/vscode/.cargo/bin:${PATH}

USER vscode
