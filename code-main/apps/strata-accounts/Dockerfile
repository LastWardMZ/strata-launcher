# Servicio de cuentas de Strata. Sin dependencias: solo Node 24+ (usa node:sqlite).
#   docker build -t strata-accounts apps/strata-accounts
#   docker run -d --name strata-accounts -p 127.0.0.1:32145:32145 -v strata-accounts-data:/data strata-accounts
# Pon delante un proxy con HTTPS (Caddy, nginx, Cloudflare…): la app solo admite https:// fuera de este equipo.
FROM node:24-slim
WORKDIR /app
COPY server.mjs ./
ENV HOST=0.0.0.0 \
    PORT=32145 \
    STRATA_ACCOUNTS_DB=/data/strata-accounts.sqlite \
    STRATA_TRUST_PROXY=1
VOLUME /data
EXPOSE 32145
USER node
HEALTHCHECK --interval=30s --timeout=5s CMD node -e "fetch('http://127.0.0.1:'+process.env.PORT+'/health').then(r=>process.exit(r.ok?0:1),()=>process.exit(1))"
CMD ["node", "server.mjs"]
