FROM alpine:latest
RUN find -type f > files.txt || true

