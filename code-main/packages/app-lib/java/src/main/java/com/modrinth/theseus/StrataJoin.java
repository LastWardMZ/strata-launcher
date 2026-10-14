package com.modrinth.theseus;

import java.lang.reflect.Constructor;
import java.lang.reflect.Method;

/**
 * Hace que un Minecraft ya abierto se conecte a un servidor (el mundo LAN de un amigo), sin cerrarlo ni reabrirlo.
 *
 * <p>Reproduce «Guardar y salir» seguido de «Conectar a un servidor» y, como {@link StrataChat}, usa reflexión sobre las
 * clases del propio juego: solo vale donde esas clases están a la vista (vanilla 26.x). Si no encajan devuelve
 * {@code false} y el launcher le explica al jugador cómo entrar a mano.
 */
public final class StrataJoin {
    private StrataJoin() {}

    /** {@code true} si la conexión quedó programada en el hilo principal del juego. */
    public static boolean join(String host, Integer port) {
        if (host == null || host.isEmpty() || port == null || port < 1 || port > 65535) {
            return false;
        }
        try {
            final Class<?> minecraftClass = Class.forName("net.minecraft.client.Minecraft");
            final Object minecraft = minecraftClass.getMethod("getInstance").invoke(null);
            if (minecraft == null) {
                return false;
            }
            final Method connect = findStartConnecting();
            final Class<?> titleScreen = Class.forName("net.minecraft.client.gui.screens.TitleScreen");
            final Constructor<?> newTitle = titleScreen.getConstructor();
            final Class<?> addressClass = Class.forName("net.minecraft.client.multiplayer.resolver.ServerAddress");
            final Class<?> dataClass = Class.forName("net.minecraft.client.multiplayer.ServerData");
            final Class<?> typeClass = Class.forName("net.minecraft.client.multiplayer.ServerData$Type");
            final Object address =
                    addressClass.getConstructor(String.class, int.class).newInstance(host, port);
            final String text = host + ":" + port;
            final Object data = dataClass
                    .getConstructor(String.class, String.class, typeClass)
                    .newInstance("Strata", text, typeClass.getField("OTHER").get(null));
            final boolean inWorld = minecraftClass.getField("level").get(minecraft) != null;
            final Method disconnect = inWorld
                    ? minecraftClass.getMethod(
                            "disconnectFromWorld", Class.forName("net.minecraft.network.chat.Component"))
                    : null;
            final Object quitMessage = inWorld
                    ? Class.forName("net.minecraft.client.multiplayer.ClientLevel")
                            .getField("DEFAULT_QUIT_MESSAGE")
                            .get(null)
                    : null;

            final Runnable task = () -> {
                try {
                    if (disconnect != null) {
                        // Igual que «Guardar y salir»: guarda el mundo y deja el menú principal.
                        disconnect.invoke(minecraft, quitMessage);
                    }
                    connect.invoke(null, newTitle.newInstance(), minecraft, address, data, false, null);
                } catch (ReflectiveOperationException | RuntimeException e) {
                    e.printStackTrace();
                }
            };
            minecraftClass.getMethod("execute", Runnable.class).invoke(minecraft, task);
            return true;
        } catch (ReflectiveOperationException | RuntimeException | LinkageError e) {
            return false;
        }
    }

    /** {@code ConnectScreen.startConnecting(Screen, Minecraft, ServerAddress, ServerData, boolean, TransferState)}. */
    private static Method findStartConnecting() throws ReflectiveOperationException {
        final Class<?> screen = Class.forName("net.minecraft.client.gui.screens.ConnectScreen");
        for (final Method method : screen.getMethods()) {
            if (method.getName().equals("startConnecting") && method.getParameterTypes().length == 6) {
                return method;
            }
        }
        throw new NoSuchMethodException("ConnectScreen.startConnecting");
    }
}
