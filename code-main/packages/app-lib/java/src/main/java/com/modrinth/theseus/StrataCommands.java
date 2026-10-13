package com.modrinth.theseus;

import com.google.gson.reflect.TypeToken;
import com.modrinth.theseus.rpc.TheseusRpc;
import java.lang.reflect.Method;
import java.lang.reflect.Proxy;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.Collections;
import java.util.List;
import java.util.Locale;

/**
 * El comando {@code /strata <amigo> <texto>}: escribe a un amigo de Strata desde el chat de Minecraft.
 *
 * <p>Minecraft envía al servidor todo lo que empieza por {@code /}, así que el comando se corta antes de salir
 * ({@link #intercept}, llamado desde {@code ClientPacketListener.sendCommand}) y se añade a la lista de comandos del
 * cliente ({@link #install}, llamado al recibirla) para que funcione el Tab. Si el servidor ya tiene un comando
 * {@code strata}, no se instala ni se intercepta nada: gana el del servidor.
 */
public final class StrataCommands {
    private static final String LITERAL = "strata";
    private static final TypeToken<String[]> STRINGS = TypeToken.get(String[].class);

    private static volatile boolean active;
    private static volatile List<String> friends = Collections.emptyList();

    private StrataCommands() {}

    /** {@code true} si el comando era de Strata y ya está tratado: no debe enviarse al servidor. */
    public static boolean intercept(String command) {
        if (!active || !isStrata(command)) {
            return false;
        }
        try {
            run(command.trim().substring(LITERAL.length()).trim());
        } catch (Throwable t) {
            StrataChat.show("No enviado", "No se pudo usar el comando de Strata.");
        }
        return true;
    }

    static boolean isStrata(String command) {
        if (command == null) {
            return false;
        }
        final String trimmed = command.trim();
        return trimmed.equalsIgnoreCase(LITERAL)
                || (trimmed.length() > LITERAL.length()
                        && trimmed.regionMatches(true, 0, LITERAL, 0, LITERAL.length())
                        && Character.isWhitespace(trimmed.charAt(LITERAL.length())));
    }

    private static void run(String rest) {
        int split = 0;
        while (split < rest.length() && !Character.isWhitespace(rest.charAt(split))) {
            split++;
        }
        final String friend = rest.substring(0, split);
        final String text = rest.substring(split).trim();
        if (friend.isEmpty() || text.isEmpty()) {
            StrataChat.show("Uso", "/strata <amigo> <mensaje>");
            return;
        }
        TheseusRpc.getRpc().callMethod(STRINGS, "strata_send", friend, text).whenComplete((result, error) -> {
            if (error == null && result != null && result.length >= 2 && "ok".equals(result[0])) {
                StrataChat.show("Enviado a " + result[1], text);
            } else {
                final String reason =
                        error != null || result == null || result.length < 2 ? "Strata no respondió." : result[1];
                StrataChat.show("No enviado", reason);
            }
        });
    }

    /** Pide a Strata la lista de amigos (nombres tal y como se escriben en el comando). */
    private static void refreshFriends() {
        try {
            TheseusRpc.getRpc().callMethod(STRINGS, "strata_friends").whenComplete((names, error) -> {
                if (error == null && names != null) {
                    friends = Collections.unmodifiableList(new ArrayList<>(Arrays.asList(names)));
                }
            });
        } catch (RuntimeException ignored) {
            // Sin conexión con el launcher: se queda la lista anterior.
        }
    }

    /** Sugerencias de Tab para el amigo; {@code builder} es un {@code SuggestionsBuilder} de Brigadier. */
    static Object suggest(Object builder) throws ReflectiveOperationException {
        refreshFriends();
        final Class<?> type = builder.getClass();
        final String typed = ((String) type.getMethod("getRemaining").invoke(builder)).toLowerCase(Locale.ROOT);
        final Method suggest = type.getMethod("suggest", String.class);
        for (final String name : friends) {
            if (name.toLowerCase(Locale.ROOT).startsWith(typed)) {
                suggest.invoke(builder, name);
            }
        }
        return type.getMethod("buildFuture").invoke(builder);
    }

    /**
     * Añade {@code strata <amigo> <mensaje>} a la lista de comandos del cliente. {@code listener} es el
     * {@code ClientPacketListener}; nunca lanza: si algo no encaja, el comando simplemente no existe.
     */
    public static void install(Object listener) {
        try {
            final Object dispatcher =
                    listener.getClass().getMethod("getCommands").invoke(listener);
            final Object root = dispatcher.getClass().getMethod("getRoot").invoke(dispatcher);
            if (root.getClass().getMethod("getChild", String.class).invoke(root, LITERAL) != null) {
                active = false;
                return;
            }
            final ClassLoader loader = dispatcher.getClass().getClassLoader();
            final Class<?> literalBuilder = brigadier(loader, "builder.LiteralArgumentBuilder");
            final Class<?> requiredBuilder = brigadier(loader, "builder.RequiredArgumentBuilder");
            final Class<?> argumentBuilder = brigadier(loader, "builder.ArgumentBuilder");
            final Class<?> argumentType = brigadier(loader, "arguments.ArgumentType");
            final Class<?> strings = brigadier(loader, "arguments.StringArgumentType");
            final Class<?> commandType = brigadier(loader, "Command");
            final Class<?> providerType = brigadier(loader, "suggestion.SuggestionProvider");
            final Class<?> nodeType = brigadier(loader, "tree.CommandNode");

            final Method argument = requiredBuilder.getMethod("argument", String.class, argumentType);
            final Object friendArgument =
                    argument.invoke(null, "amigo", strings.getMethod("word").invoke(null));
            final Object messageArgument = argument.invoke(
                    null, "mensaje", strings.getMethod("greedyString").invoke(null));

            final Object command =
                    Proxy.newProxyInstance(loader, new Class<?>[] {commandType}, (proxy, method, args) -> {
                        if (method.getName().equals("run")) {
                            return 1;
                        }
                        return objectMethod(proxy, method.getName(), args);
                    });
            final Object provider =
                    Proxy.newProxyInstance(loader, new Class<?>[] {providerType}, (proxy, method, args) -> {
                        if (method.getName().equals("getSuggestions")) {
                            return suggest(args[1]);
                        }
                        return objectMethod(proxy, method.getName(), args);
                    });

            requiredBuilder.getMethod("executes", commandType).invoke(messageArgument, command);
            requiredBuilder.getMethod("suggests", providerType).invoke(friendArgument, provider);
            final Method then = argumentBuilder.getMethod("then", argumentBuilder);
            then.invoke(friendArgument, messageArgument);
            final Object literal =
                    literalBuilder.getMethod("literal", String.class).invoke(null, LITERAL);
            then.invoke(literal, friendArgument);
            final Object node = argumentBuilder.getMethod("build").invoke(literal);
            root.getClass().getMethod("addChild", nodeType).invoke(root, node);
            active = true;
            refreshFriends();
        } catch (ReflectiveOperationException | RuntimeException | LinkageError e) {
            active = false;
        }
    }

    private static Class<?> brigadier(ClassLoader loader, String name) throws ClassNotFoundException {
        return Class.forName("com.mojang.brigadier." + name, true, loader);
    }

    private static Object objectMethod(Object proxy, String name, Object[] args) {
        switch (name) {
            case "hashCode":
                return System.identityHashCode(proxy);
            case "equals":
                return proxy == args[0];
            case "toString":
                return "StrataCommands";
            default:
                return null;
        }
    }
}
