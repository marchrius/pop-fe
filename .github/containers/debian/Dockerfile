FROM ubuntu:24.04

ENV DEBIAN_FRONTEND=noninteractive

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        build-essential \
        ca-certificates \
        cmake \
        curl \
        dpkg-dev \
        ffmpeg \
        git \
        libadwaita-1-dev \
        libclang-dev \
        libgtk-4-dev \
        libopencv-contrib-dev \
        libopencv-dev \
        libsndfile1-dev \
        libssl-dev \
        mame-tools \
        pkg-config \
        p7zip-full \
        poppler-utils \
        python3 \
        tar \
        xdelta3 \
    && rm -rf /var/lib/apt/lists/*
