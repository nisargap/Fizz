FROM public.ecr.aws/docker/library/rust:1.94-bookworm AS build
WORKDIR /build
COPY server.rs .
RUN rustc --edition=2024 -O server.rs -o fizz-worker

FROM public.ecr.aws/docker/library/debian:bookworm-slim
COPY --from=build /build/fizz-worker /usr/local/bin/fizz-worker
USER 65532:65532
CMD ["/usr/local/bin/fizz-worker"]
