package com.modrinth.theseus;

import java.lang.reflect.Field;
import java.lang.reflect.Method;

/**
 * Muestra mensajes de Strata (amigos, invitaciones, avisos) en el chat de Minecraft. Es un mensaje solo local: no se envía
 * al servidor ni lo ve nadie más.
 *
 * <p>Usa reflexión sobre las clases del propio juego, así que no depende de ningún mod y funciona en instancias vanilla.
 * Si las clases no están a la vista (otro cargador como Fabric o Forge, o una versión con nombres ofuscados) simplemente
 * devuelve {@code false} y el launcher recurre a su aviso sobre la ventana.
 */
public final class StrataChat {
    private static final int MAX_TITLE = 32;
    private static final int MAX_TEXT = 200;

    private StrataChat() {}

    /** {@code true} si el mensaje se puso en cola para mostrarse en el chat. */
    public static boolean show(String title, String text) {
        try {
            final Class<?> minecraftClass = Class.forName("net.minecraft.client.Minecraft");
            final Object minecraft = minecraftClass.getMethod("getInstance").invoke(null);
            // Sin jugador (pantalla de título, cargando) no hay chat al que escribir.
            if (minecraft == null || field(minecraft, "player") == null) {
                return false;
            }
            final Object chat = findChat(minecraft);
            final Class<?> componentClass = Class.forName("net.minecraft.network.chat.Component");
            final Object component =
                    componentClass.getMethod("literal", String.class).invoke(null, format(title, text));
            final Method add = findAddMethod(chat, component);
            if (add == null) {
                return false;
            }
            // El chat solo se toca desde el hilo principal del juego.
            final Runnable task = () -> {
                try {
                    add.invoke(chat, component);
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

    /** {@code §b[Strata] §f<título>§7: §f<texto>} con el texto limpio de códigos de formato y saltos de línea. */
    static String format(String title, String text) {
        final String cleanTitle = clean(title, MAX_TITLE);
        final String cleanText = clean(text, MAX_TEXT);
        final StringBuilder line = new StringBuilder("§b[Strata] §f").append(cleanTitle);
        if (!cleanText.isEmpty()) {
            line.append("§7: §f").append(cleanText);
        }
        return line.toString();
    }

    /** Quita los códigos de formato (§) y los caracteres de control, y recorta: quien escribe no puede colorear ni inyectar. */
    static String clean(String text, int max) {
        if (text == null) {
            return "";
        }
        final StringBuilder out = new StringBuilder();
        for (int i = 0; i < text.length() && out.length() < max; i++) {
            final char c = text.charAt(i);
            out.append(c == '§' || c < ' ' ? ' ' : c);
        }
        return out.toString().trim().replaceAll(" {2,}", " ");
    }

    /** {@code Minecraft.gui[.hud].getChat()}: en versiones recientes el chat cuelga de un {@code Hud}. */
    private static Object findChat(Object minecraft) throws ReflectiveOperationException {
        final Object gui = field(minecraft, "gui");
        Object holder = gui;
        try {
            holder = field(gui, "hud");
        } catch (NoSuchFieldException ignored) {
            // Versiones anteriores: el chat se pide directamente al Gui.
        }
        return holder.getClass().getMethod("getChat").invoke(holder);
    }

    private static Method findAddMethod(Object chat, Object component) {
        for (final String name : new String[] {"addClientSystemMessage", "addMessage"}) {
            for (final Method method : chat.getClass().getMethods()) {
                final Class<?>[] parameters = method.getParameterTypes();
                if (method.getName().equals(name) && parameters.length == 1 && parameters[0].isInstance(component)) {
                    return method;
                }
            }
        }
        return null;
    }

    private static Object field(Object target, String name) throws ReflectiveOperationException {
        final Field field = target.getClass().getField(name);
        return field.get(target);
    }
}
