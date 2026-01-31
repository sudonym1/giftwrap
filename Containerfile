FROM alpine:latest
RUN apk add --no-cache sudo
RUN find -type f > files.txt || true
