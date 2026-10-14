package com.modrinth.theseus;

import java.lang.reflect.Method;
import java.util.concurrent.CompletableFuture;
import java.util.concurrent.TimeUnit;

/**
 * Abre a LAN el mundo en el que está el jugador (lo mismo que «Abrir a LAN» del menú de pausa), para poder invitar a un
 * amigo de un clic. Usa reflexión sobre las clases del juego: solo vale en vanilla 26.x.
 */
public final class StrataLan {
    private static final int FAILED = -1;

    private StrataLan() {}

    /**
     * Puerto LAN del mundo actual (lo abre si hace falta) o {@code -1} si no se pudo: no hay mundo propio abierto, o las
     * clases del juego no son las esperadas.
     */
    public static Integer open() {
        try {
            final Class<?> minecraftClass = Class.forName("net.minecraft.client.Minecraft");
            final Object minecraft = minecraftClass.getMethod("getInstance").invoke(null);
            if (minecraft == null) {
                return FAILED;
            }
            final Object server =
                    minecraftClass.getMethod("getSingleplayerServer").invoke(minecraft);
            if (server == null) {
                return FAILED;
            }
            final CompletableFuture<Integer> result = new CompletableFuture<>();
            final Runnable task = () -> {
                try {
                    result.complete(publish(server));
                } catch (ReflectiveOperationException | RuntimeException e) {
                    result.complete(FAILED);
                }
            };
            // El servidor integrado solo se toca desde el hilo principal del juego.
            minecraftClass.getMethod("execute", Runnable.class).invoke(minecraft, task);
            return result.get(5, TimeUnit.SECONDS);
        } catch (ReflectiveOperationException | RuntimeException | LinkageError e) {
            return FAILED;
        } catch (java.util.concurrent.ExecutionException | java.util.concurrent.TimeoutException e) {
            return FAILED;
        } catch (InterruptedException e) {
            Thread.currentThread().interrupt();
            return FAILED;
        }
    }

    private static int publish(Object server) throws ReflectiveOperationException {
        final Class<?> type = server.getClass();
        if ((Boolean) type.getMethod("isPublished").invoke(server)) {
            return (Integer) type.getMethod("getPort").invoke(server);
        }
        final Class<?> scope = Class.forName("net.minecraft.server.MinecraftServer$MultiplayerScope");
        final Object lan = scope.getField("LAN").get(null);
        final int port = (Integer) Class.forName("net.minecraft.util.HttpUtil")
                .getMethod("getAvailablePort")
                .invoke(null);
        final Method publish = type.getMethod("publishServer", scope, int.class);
        return (Boolean) publish.invoke(server, lan, port) ? port : FAILED;
    }
}
