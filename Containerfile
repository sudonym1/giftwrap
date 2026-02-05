FROM alpine:latest
RUN apk add --no-cache sudo
COPY stunt_file /stunt
