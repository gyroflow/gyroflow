FROM mcr.microsoft.com/devcontainers/rust:1-1-bullseye

RUN apt-get update \
 && DEBIAN_FRONTEND=noninteractive apt-get -y install --no-install-recommends \
        p7zip-full \
 && rm -rf /var/lib/apt/lists/*

ARG USERNAME=vscode
USER $USERNAME

# Save command line history
RUN echo "export HISTFILE=/home/$USERNAME/commandhistory/.bash_history" >> "/home/$USERNAME/.bashrc" \
        && echo "export PROMPT_COMMAND='history -a'" >> "/home/$USERNAME/.bashrc" \
        && mkdir -p /home/$USERNAME/commandhistory \
        && touch /home/$USERNAME/commandhistory/.bash_history \
        && chown -R $USERNAME /home/$USERNAME/commandhistory
