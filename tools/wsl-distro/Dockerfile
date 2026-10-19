# howldrop.exe's WSL distro, AirDrop on this PC (howldrop/src/wsl_win.cpp):
# the amd64 add-on image of the same version, with WSL's settings and the
# version howldrop.exe checks before using it. release-windows.yml exports
# it as howldrop-linux.tar.gz for `wsl --import`.
ARG IMAGE
FROM ${IMAGE}
ARG VERSION
COPY wsl.conf /etc/wsl.conf
RUN test -x /wsl-run.sh && printf '%s\n' "$VERSION" > /etc/howldrop-version
