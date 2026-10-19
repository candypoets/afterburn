FROM debian:bookworm-slim
RUN useradd --system --uid 10001 --home /data --shell /usr/sbin/nologin figcraft
COPY bin/figcraft-server /usr/local/bin/figcraft-server
RUN mkdir -p /data && chown 10001:10001 /data
USER 10001
EXPOSE 47910
ENTRYPOINT ["figcraft-server"]
CMD ["--listen", "0.0.0.0:47910", "--data", "/data"]
