# syntax=docker/dockerfile:1.18
# The web app (landing + hub) as a Next.js standalone server on distroless
# Node: no shell, no package manager, unprivileged user.
#
#   docker build -f deploy/docker/web.Dockerfile -t hebra-web .

FROM node:24.22.0-trixie-slim AS build
WORKDIR /src
ENV NEXT_TELEMETRY_DISABLED=1
RUN npm install -g pnpm@12.11.2
COPY package.json pnpm-lock.yaml pnpm-workspace.yaml .npmrc ./
COPY apps/web/package.json apps/web/
RUN --mount=type=cache,target=/root/.local/share/pnpm/store,sharing=locked \
    pnpm install --frozen-lockfile --filter @hebra/web
COPY apps/web apps/web
COPY docs/billing-policy.md docs/billing-policy.md
RUN pnpm --filter @hebra/web build \
 && cp -r apps/web/.next/static apps/web/.next/standalone/apps/web/.next/static \
 && cp -r apps/web/public apps/web/.next/standalone/apps/web/public

FROM gcr.io/distroless/nodejs24-debian13:nonroot
LABEL org.opencontainers.image.source="https://github.com/s-pl/hebra" \
      org.opencontainers.image.description="Hebra web: landing and hub" \
      org.opencontainers.image.licenses="Apache-2.0"
WORKDIR /app
COPY --from=build --chown=65532:65532 /src/apps/web/.next/standalone ./
USER 65532:65532
ENV NODE_ENV=production \
    NEXT_TELEMETRY_DISABLED=1 \
    HOSTNAME=0.0.0.0 \
    PORT=3000
EXPOSE 3000
CMD ["apps/web/server.js"]
