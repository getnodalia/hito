//! Interface text in Spanish and English.
//!
//! The source is written in English, through [`tr`]; Spanish comes from [`SPANISH`]. A test reads
//! every `tr("…")` in `src/` and fails when one has no Spanish entry, so a new string cannot ship
//! untranslated. Text with a value in it uses `{}` placeholders filled by [`trf`].

use std::sync::atomic::{AtomicU8, Ordering};

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Lang {
    Es,
    En,
}

impl Lang {
    /// The system language when it is Spanish, English otherwise. macOS and Windows keep the
    /// language in their own settings (an app opened from the Dock has no `LANG`), so ask them;
    /// Linux and the BSDs say it in the environment.
    pub fn system() -> Lang {
        if system_tag().is_some_and(|tag| is_spanish(&tag)) {
            Lang::Es
        } else {
            Lang::En
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Lang::Es => "Español",
            Lang::En => "English",
        }
    }
}

fn system_tag() -> Option<String> {
    let platform = || sys_locale::get_locale().filter(|tag| !tag.is_empty() && tag.len() <= 64);
    if cfg!(any(target_os = "macos", target_os = "windows")) {
        return platform();
    }
    ["LC_ALL", "LC_MESSAGES", "LANG", "LANGUAGE"]
        .iter()
        .filter_map(|v| std::env::var(v).ok())
        .find(|v| !v.is_empty())
        .or_else(platform)
}

/// Whether a locale tag (`es`, `es-ES`, `es_MX.UTF-8`, `es-419`) names Spanish.
fn is_spanish(tag: &str) -> bool {
    let tag = tag.trim().to_ascii_lowercase();
    tag.strip_prefix("es")
        .is_some_and(|rest| rest.is_empty() || rest.starts_with(['-', '_', '.', '@']))
}

static CURRENT: AtomicU8 = AtomicU8::new(0);

pub fn set(lang: Lang) {
    CURRENT.store(
        match lang {
            Lang::Es => 0,
            Lang::En => 1,
        },
        Ordering::Relaxed,
    );
}

pub fn current() -> Lang {
    match CURRENT.load(Ordering::Relaxed) {
        0 => Lang::Es,
        _ => Lang::En,
    }
}

/// The text in the current language.
pub fn tr(english: &'static str) -> &'static str {
    if current() == Lang::En {
        return english;
    }
    SPANISH
        .binary_search_by(|(en, _)| en.cmp(&english))
        .ok()
        .and_then(|i| SPANISH.get(i))
        .map_or(english, |(_, es)| es)
}

/// [`tr`] with each `{}` replaced, in order, by `args`.
pub fn trf(english: &'static str, args: &[&str]) -> String {
    let mut out = String::new();
    let mut rest = tr(english);
    for arg in args {
        match rest.split_once("{}") {
            Some((before, after)) => {
                out.push_str(before);
                out.push_str(arg);
                rest = after;
            }
            None => break,
        }
    }
    out.push_str(rest);
    out
}

/// English → Spanish, sorted by the English text (the lookup is a binary search).
const SPANISH: &[(&str, &str)] = &[
    ("1 day", "1 día"),
    ("30 days", "30 días"),
    ("7 days", "7 días"),
    ("A milestone was reached", "Se alcanzó un hito"),
    ("A short cycle with a review at the end.", "Un ciclo corto con una revisión al final."),
    ("A task was assigned to you.", "Se te ha asignado una tarea."),
    ("A token lets another app (Ágora, a script) act as you: it sees and changes what you can. Revoke it when it is no longer needed.", "Un token deja que otra app (Ágora, un script) actúe por ti: ve y cambia lo mismo que tú. Revócalo cuando ya no haga falta."),
    ("A workspace holds your company's projects and people.", "Un espacio de trabajo reúne los proyectos y las personas de tu empresa."),
    ("API tokens", "Tokens de API"),
    ("API tokens and webhooks", "Tokens de API y webhooks"),
    ("About Hito", "Acerca de Hito"),
    ("Accent, typefaces, icons and window controls come from the Nodalia Design Language.", "Acento, tipografías, iconos y controles de ventana vienen del Nodalia Design Language."),
    ("Account", "Cuenta"),
    ("Acme Inc.", "Acme S. L."),
    ("Active projects", "Proyectos activos"),
    ("Activity", "Actividad"),
    ("Add a task", "Añadir una tarea"),
    ("Add an item", "Añadir un elemento"),
    ("Add details, links or notes", "Añade detalles, enlaces o notas"),
    ("Add milestone", "Añadir hito"),
    ("Add task", "Añadir tarea"),
    ("Add webhook", "Añadir webhook"),
    ("Address", "Dirección"),
    ("Agree on the goals and the date", "Acordar los objetivos y la fecha"),
    ("An empty board.", "Un tablero vacío."),
    ("Another member's device shares them as soon as it is online. Nothing here can be read or changed until then.", "El dispositivo de otro miembro las comparte en cuanto esté conectado. Hasta entonces no se puede leer ni cambiar nada aquí."),
    ("Anyone can create an account.", "Cualquiera puede crear una cuenta."),
    ("Appearance", "Apariencia"),
    ("Archived", "Archivado"),
    ("Archived (read-only for everyone)", "Archivado (solo lectura para todos)"),
    ("Assignee", "Responsable"),
    ("Back", "Volver"),
    ("Back to sign in", "Volver a iniciar sesión"),
    ("Beta ready", "Beta lista"),
    ("Blank project", "Proyecto en blanco"),
    ("Board", "Tablero"),
    ("Book the venue", "Reservar el lugar"),
    ("Build", "Construir"),
    ("Build the first version", "Construir la primera versión"),
    ("Cancel", "Cancelar"),
    ("Checklist", "Lista de comprobación"),
    ("Client project", "Proyecto de cliente"),
    ("Client review", "Revisión del cliente"),
    ("Close", "Cerrar"),
    ("Close the task or this view", "Cerrar la tarea o esta vista"),
    ("Comment", "Comentar"),
    ("Comment added", "Comentario añadido"),
    ("Comments ({})", "Comentarios ({})"),
    ("Confirm the scope and budget", "Confirmar el alcance y el presupuesto"),
    ("Connected", "Conectado"),
    ("Connecting…", "Conectando…"),
    ("Copied.", "Copiado."),
    ("Copy", "Copiar"),
    ("Copy code", "Copiar código"),
    ("Copy it now: it is shown only once.", "Cópialo ahora: solo se muestra una vez."),
    ("Create", "Crear"),
    ("Create a project first, then add its tasks.", "Crea primero un proyecto y luego añade sus tareas."),
    ("Create a workspace", "Crear un espacio de trabajo"),
    ("Create a workspace for your company, or join one with an invitation code.", "Crea un espacio de trabajo para tu empresa o únete a uno con un código de invitación."),
    ("Create account", "Crear cuenta"),
    ("Create an account", "Crear una cuenta"),
    ("Create invitation", "Crear invitación"),
    ("Create project", "Crear proyecto"),
    ("Create token", "Crear token"),
    ("Credits", "Créditos"),
    ("Daily check-ins", "Reuniones diarias"),
    ("Dark", "Oscuro"),
    ("Delete", "Eliminar"),
    ("Delete comment", "Eliminar comentario"),
    ("Delete milestone", "Eliminar hito"),
    ("Delete task", "Eliminar tarea"),
    ("Delete this milestone?", "¿Eliminar este hito?"),
    ("Delete this task?", "¿Eliminar esta tarea?"),
    ("Delivery", "Entrega"),
    ("Demo", "Demostración"),
    ("Description", "Descripción"),
    ("Disconnected. Retrying in {} s.", "Sin conexión. Reintentando en {} s."),
    ("Display name", "Nombre visible"),
    ("Done", "Hecho"),
    ("Done across projects", "Hecho en todos los proyectos"),
    ("Due date", "Fecha límite"),
    ("Due soon", "Vence pronto"),
    ("Edit milestone", "Editar hito"),
    ("Event", "Evento"),
    ("Every project", "Todos los proyectos"),
    ("Everything", "Todo"),
    ("Everything assigned to you, in every project.", "Todo lo que tienes asignado, en todos los proyectos."),
    ("File", "Archivo"),
    ("Filter tasks", "Filtrar tareas"),
    ("Filter tasks by title or label", "Filtrar tareas por título o etiqueta"),
    ("Final changes", "Cambios finales"),
    ("First delivery", "Primera entrega"),
    ("From the kickoff to the handover.", "Del arranque a la entrega final."),
    ("Guest", "Invitado"),
    ("Guests cannot create projects. Ask an admin of the workspace.", "Los invitados no pueden crear proyectos. Pídeselo a un administrador del espacio."),
    ("Guests see only the projects you add them to, and can comment there.", "Los invitados solo ven los proyectos a los que los añadas, y pueden comentar en ellos."),
    ("Guests who can see this project", "Invitados que pueden ver este proyecto"),
    ("Handover", "Entrega final"),
    ("Hello, {}", "Hola, {}"),
    ("Hello, {}.", "Hola, {}."),
    ("High", "Alta"),
    ("High contrast", "Alto contraste"),
    ("Hito posts each change to the address, signed with a secret. In Ágora, add an incoming Hito webhook to a channel and paste its address here.", "Hito envía cada cambio a la dirección, firmado con un secreto. En Ágora, añade a un canal un webhook entrante de Hito y pega aquí su dirección."),
    ("Home", "Inicio"),
    ("How many people may use it", "Cuántas personas pueden usarla"),
    ("In progress", "En curso"),
    ("In review", "En revisión"),
    ("Interface language", "Idioma de la interfaz"),
    ("Invitation code", "Código de invitación"),
    ("Invitation code (if you have one)", "Código de invitación (si tienes uno)"),
    ("Invite people", "Invitar a gente"),
    ("Its tasks stay in the project, without a milestone. This cannot be undone.", "Sus tareas se quedan en el proyecto, sin hito. No se puede deshacer."),
    ("Join", "Unirse"),
    ("Join a workspace", "Unirse a un espacio de trabajo"),
    ("Keep me signed in on this computer", "Mantener la sesión en este ordenador"),
    ("Keyboard shortcuts", "Atajos de teclado"),
    ("Kickoff", "Arranque"),
    ("Kickoff meeting", "Reunión de arranque"),
    ("Labels", "Etiquetas"),
    ("Language", "Idioma"),
    ("Later", "Más adelante"),
    ("Launch", "Lanzamiento"),
    ("Light", "Claro"),
    ("List", "Lista"),
    ("Log out", "Cerrar sesión"),
    ("Low", "Baja"),
    ("Main menu", "Menú principal"),
    ("Mark as done", "Marcar como hecha"),
    ("Maximize", "Maximizar"),
    ("Member", "Miembro"),
    ("Members see every project and work on every task.", "Los miembros ven todos los proyectos y trabajan en todas las tareas."),
    ("Milestone", "Hito"),
    ("Milestone added", "Hito añadido"),
    ("Milestone changed", "Hito cambiado"),
    ("Milestone deleted", "Hito eliminado"),
    ("Milestone reached", "Hito alcanzado"),
    ("Milestones", "Hitos"),
    ("Milestones mark the points that matter: a delivery, a launch, a review.", "Los hitos marcan los momentos que importan: una entrega, un lanzamiento, una revisión."),
    ("Minimize", "Minimizar"),
    ("My tasks", "Mis tareas"),
    ("Name", "Nombre"),
    ("New milestone", "Nuevo hito"),
    ("New project", "Nuevo proyecto"),
    ("New task", "Nueva tarea"),
    ("No date", "Sin fecha"),
    ("No milestone", "Sin hito"),
    ("No projects yet.", "Aún no hay proyectos."),
    ("No projects yet. Create the first one with +.", "Aún no hay proyectos. Crea el primero con +."),
    ("No projects yet. Start one from a template above.", "Aún no hay proyectos. Empieza uno con una plantilla de arriba."),
    ("No projects you can see yet.", "Aún no hay proyectos que puedas ver."),
    ("No tasks here yet.", "Aún no hay tareas aquí."),
    ("Nobody", "Nadie"),
    ("Nodalia", "Nodalia"),
    ("Normal", "Normal"),
    ("Nothing assigned to you is open. Well done.", "No tienes nada asignado pendiente. Buen trabajo."),
    ("Nothing has happened here yet.", "Aquí aún no ha pasado nada."),
    ("Nothing of yours is due in the next seven days.", "No vence nada tuyo en los próximos siete días."),
    ("Notifications", "Notificaciones"),
    ("Notifications are not available on this desktop.", "Las notificaciones no están disponibles en este escritorio."),
    ("Notifications never include a task's title or its project.", "Las notificaciones nunca incluyen el título de la tarea ni su proyecto."),
    ("Of yours, overdue", "Tuyas, vencidas"),
    ("Only people with an invitation can register.", "Solo puede registrarse quien tenga invitación."),
    ("Only the workspace's owners and admins manage webhooks.", "Solo los propietarios y administradores del espacio gestionan los webhooks."),
    ("Open tasks for you", "Tareas pendientes para ti"),
    ("Order the catering", "Encargar el catering"),
    ("Overdue", "Vencidas"),
    ("Overdue in the workspace", "Vencidas en el espacio"),
    ("Overview", "Resumen"),
    ("Password", "Contraseña"),
    ("Paste the invitation code someone in the workspace sent you.", "Pega el código de invitación que te envió alguien del espacio."),
    ("Plan", "Planificar"),
    ("Plan, build, launch: three milestones.", "Planificar, construir, lanzar: tres hitos."),
    ("Preferences", "Preferencias"),
    ("Preparation", "Preparación"),
    ("Prepare the announcement", "Preparar el anuncio"),
    ("Priority", "Prioridad"),
    ("Privacy", "Privacidad"),
    ("Product launch", "Lanzamiento de producto"),
    ("Progress", "Progreso"),
    ("Project", "Proyecto"),
    ("Project archived", "Proyecto archivado"),
    ("Project changed", "Proyecto cambiado"),
    ("Project created", "Proyecto creado"),
    ("Project settings", "Ajustes del proyecto"),
    ("Project templates are written in the language chosen when the project is created.", "Las plantillas de proyecto se escriben en el idioma elegido al crear el proyecto."),
    ("Projects", "Proyectos"),
    ("Quit", "Salir"),
    ("Recent projects", "Proyectos recientes"),
    ("Reduce motion", "Reducir el movimiento"),
    ("Refresh", "Actualizar"),
    ("Registration is closed on this server.", "El registro está cerrado en este servidor."),
    ("Remove", "Quitar"),
    ("Restore", "Restaurar"),
    ("Retrospective", "Retrospectiva"),
    ("Revoke", "Revocar"),
    ("Save", "Guardar"),
    ("Save the open task", "Guardar la tarea abierta"),
    ("Saving…", "Guardando…"),
    ("Send", "Enviar"),
    ("Send the invitations", "Enviar las invitaciones"),
    ("Send the invoice", "Enviar la factura"),
    ("Send this code to the people you are inviting. It is shown only once.", "Envía este código a las personas que invitas. Solo se muestra una vez."),
    ("Server", "Servidor"),
    ("Set up the room", "Preparar la sala"),
    ("Show older activity", "Mostrar actividad anterior"),
    ("Sign in", "Iniciar sesión"),
    ("Sign in to Hito", "Inicia sesión en Hito"),
    ("Signed out. Sign in again.", "Sesión cerrada. Vuelve a iniciar sesión."),
    ("Signing key (optional)", "Clave de firma (opcional)"),
    ("Someone", "Alguien"),
    ("Someone changed this task while you were editing it. It shows their version now.", "Alguien cambió esta tarea mientras la editabas. Ahora se muestra su versión."),
    ("Sprint planning", "Planificación del sprint"),
    ("Sprint review", "Revisión del sprint"),
    ("Start a project", "Empezar un proyecto"),
    ("Start from", "Empezar desde"),
    ("Status", "Estado"),
    ("Stay signed in on this computer", "Mantener la sesión iniciada en este ordenador"),
    ("System", "Sistema"),
    ("Task", "Tarea"),
    ("Task added", "Tarea añadida"),
    ("Task assigned", "Tarea asignada"),
    ("Task changed", "Tarea cambiada"),
    ("Task completed", "Tarea completada"),
    ("Task deleted", "Tarea eliminada"),
    ("Task moved", "Tarea movida"),
    ("Task reopened", "Tarea reabierta"),
    ("Task titles, descriptions, checklists, labels, comments and milestone names are end-to-end encrypted: only your team's devices can read them. The server sees the rest: project names, who is assigned, statuses, priorities, dates and the activity log.", "Los títulos, descripciones, listas, etiquetas y comentarios de las tareas, y los nombres de los hitos, están cifrados de extremo a extremo: solo los dispositivos de tu equipo pueden leerlos. El servidor ve el resto: los nombres de los proyectos, a quién se asigna cada tarea, estados, prioridades, fechas y el registro de actividad."),
    ("Tasks, milestones and progress for your company, on a server you run yourself.", "Tareas, hitos y progreso para tu empresa, en un servidor que gestionas tú."),
    ("Test with five customers", "Probar con cinco clientes"),
    ("Thank the guests", "Agradecer a los invitados"),
    ("That project is no longer available.", "Ese proyecto ya no está disponible."),
    ("The day", "El día"),
    ("The desktop's own", "Los del escritorio"),
    ("The project's description, milestones and tasks are end-to-end encrypted.", "La descripción, los hitos y las tareas del proyecto están cifrados de extremo a extremo."),
    ("The task, its checklist and its comments are deleted for everyone. This cannot be undone.", "La tarea, su lista y sus comentarios se eliminan para todos. No se puede deshacer."),
    ("Theme", "Tema"),
    ("They join as", "Se unen como"),
    ("This address uses http: passwords and everything the server sees travel unencrypted. Use it only on your own computer or network.", "Esta dirección usa http: las contraseñas y todo lo que ve el servidor viajan sin cifrar. Úsala solo en tu ordenador o tu red."),
    ("This device does not have this project's keys yet. They arrive when a teammate with access opens Hito.", "Este dispositivo aún no tiene las claves de este proyecto. Llegan cuando un compañero con acceso abre Hito."),
    ("This week", "Esta semana"),
    ("Title", "Título"),
    ("To do", "Por hacer"),
    ("Today", "Hoy"),
    ("Token name, e.g. Ágora", "Nombre del token, p. ej. Ágora"),
    ("Two-week sprint", "Sprint de dos semanas"),
    ("Unsaved changes", "Cambios sin guardar"),
    ("Urgent", "Urgente"),
    ("Username", "Usuario"),
    ("Valid for", "Válida durante"),
    ("Venue, guests and the day itself.", "El lugar, los invitados y el propio día."),
    ("Version {}", "Versión {}"),
    ("View", "Ver"),
    ("Waiting for this project's keys", "Esperando las claves de este proyecto"),
    ("Webhooks", "Webhooks"),
    ("Website relaunch", "Renovación de la web"),
    ("What needs doing", "Qué hay que hacer"),
    ("What reaching it means (optional)", "Qué significa alcanzarlo (opcional)"),
    ("What this project is for (optional)", "Para qué es este proyecto (opcional)"),
    ("When a task is assigned to me", "Cuando se me asigna una tarea"),
    ("Window", "Ventana"),
    ("Window buttons", "Botones de la ventana"),
    ("Workspaces", "Espacios de trabajo"),
    ("Write a comment", "Escribe un comentario"),
    ("Write the brief", "Redactar el briefing"),
    ("Write the date as YYYY-MM-DD.", "Escribe la fecha como AAAA-MM-DD."),
    ("design, backend", "diseño, backend"),
    ("never used", "nunca usado"),
    ("used {}", "usado {}"),
    ("{} % done · {}", "{} % hecho · {}"),
    ("{} added a milestone", "{} añadió un hito"),
    ("{} added a task", "{} añadió una tarea"),
    ("{} archived the project", "{} archivó el proyecto"),
    ("{} assigned a task", "{} asignó una tarea"),
    ("{} changed a milestone", "{} cambió un hito"),
    ("{} changed a task", "{} cambió una tarea"),
    ("{} changed the project", "{} cambió el proyecto"),
    ("{} commented on a task", "{} comentó una tarea"),
    ("{} completed a task", "{} completó una tarea"),
    ("{} created the project", "{} creó el proyecto"),
    ("{} deleted a milestone", "{} eliminó un hito"),
    ("{} deleted a task", "{} eliminó una tarea"),
    ("{} moved a task", "{} movió una tarea"),
    ("{} of {} done", "{} de {} hechas"),
    ("{} of {} tasks done", "{} de {} tareas hechas"),
    ("{} overdue", "{} vencidas"),
    ("{} reopened a task", "{} reabrió una tarea"),
    ("{} · up to {} people · until {}", "{} · hasta {} personas · hasta el {}"),
    ("Ágora #product", "Ágora #producto"),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spanish_locale_tags_are_recognised() {
        for tag in ["es", "es-ES", "es_ES.UTF-8", "es-419", "ES_mx", "es@euro"] {
            assert!(is_spanish(tag), "{tag}");
        }
        for tag in ["en-US", "en_GB.UTF-8", "et-EE", "", "C", "POSIX", "est"] {
            assert!(!is_spanish(tag), "{tag}");
        }
    }

    #[test]
    fn the_table_is_sorted_without_duplicates() {
        for pair in SPANISH.windows(2) {
            if let [(a, _), (b, _)] = pair {
                assert!(a < b, "{a:?} must come before {b:?} and appear once");
            }
        }
    }

    #[test]
    fn every_interface_string_has_spanish() {
        let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut missing = Vec::new();
        let mut stack = vec![src];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                if path.file_name().is_some_and(|n| n == "i18n.rs") {
                    continue;
                }
                let text = std::fs::read_to_string(&path).unwrap();
                for marker in ["tr(", "trf("] {
                    for (at, _) in text.match_indices(marker) {
                        let before = text[..at].chars().next_back();
                        if before.is_some_and(|c| c.is_alphanumeric() || c == '_') {
                            continue;
                        }
                        let rest = text[at + marker.len()..].trim_start();
                        let Some(piece) = rest.strip_prefix('"') else {
                            continue;
                        };
                        let Some(end) = piece.find('"') else {
                            continue;
                        };
                        let english = &piece[..end];
                        if SPANISH
                            .binary_search_by(|(en, _)| en.cmp(&english))
                            .is_err()
                        {
                            missing.push(format!("{}: {english}", path.display()));
                        }
                    }
                }
            }
        }
        missing.sort();
        missing.dedup();
        assert!(
            missing.is_empty(),
            "untranslated strings:\n{}",
            missing.join("\n")
        );
    }

    #[test]
    fn placeholders_fill_in_order() {
        set(Lang::Es);
        assert_eq!(trf("{} of {} done", &["3", "5"]), "3 de 5 hechas");
        assert_eq!(tr("not in the table"), "not in the table");
        set(Lang::En);
        assert_eq!(trf("Hello, {}.", &["Ada"]), "Hello, Ada.");
        set(Lang::Es);
    }
}
