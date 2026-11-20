//! Vanilla `/effect` command support.

use steel_registry::{REGISTRY, RegistryExt as _, mob_effect::MobEffectRef};
use steel_utils::{Identifier, translations};
use text_components::TextComponent;

use super::super::{
    brigadier::{ArgumentType, CommandNodeBuilder, CommandSyntaxError},
    execution::{
        CommandSource, SteelArgumentType, SteelCommandContext, SteelCommandRuntime, argument,
        literal,
    },
    registration::CommandRegistration,
};
use crate::{
    behavior::MOB_EFFECT_BEHAVIORS,
    entity::{MobEffectInstance, SharedEntity},
};

pub(super) fn registration() -> CommandRegistration<CommandSource> {
    CommandRegistration::new(Identifier::vanilla_static("effect"), |_| command())
}

fn command() -> CommandNodeBuilder<CommandSource, SteelCommandRuntime> {
    literal("effect")
        .then(
            literal("give").then(
                argument("targets", SteelArgumentType::entities()).then(
                    argument("effect", SteelArgumentType::mob_effect())
                        .executes(give)
                        .then(
                            argument("seconds", ArgumentType::integer(1, 1_000_000))
                                .executes(give)
                                .then(
                                    argument("amplifier", ArgumentType::integer(0, 255))
                                        .executes(give)
                                        .then(
                                            argument("hideParticles", ArgumentType::bool())
                                                .executes(give),
                                        ),
                                ),
                        )
                        .then(
                            literal("infinite").executes(give_infinite).then(
                                argument("amplifier", ArgumentType::integer(0, 255))
                                    .executes(give_infinite)
                                    .then(
                                        argument("hideParticles", ArgumentType::bool())
                                            .executes(give_infinite),
                                    ),
                            ),
                        ),
                ),
            ),
        )
        .then(
            literal("clear").executes(clear_self).then(
                argument("targets", SteelArgumentType::entities())
                    .executes(clear)
                    .then(argument("effect", SteelArgumentType::mob_effect()).executes(clear)),
            ),
        )
}

fn parse_effect(
    context: &SteelCommandContext<CommandSource>,
) -> Result<MobEffectRef, CommandSyntaxError> {
    let key = context.identifier("effect")?;
    REGISTRY.mob_effects.by_key(key).ok_or_else(|| {
        CommandSyntaxError::dynamic(TextComponent::from(format!("Unknown effect: {key}")))
    })
}

fn give(context: &SteelCommandContext<CommandSource>) -> Result<i32, CommandSyntaxError> {
    give_with_duration(context, false)
}

fn give_infinite(context: &SteelCommandContext<CommandSource>) -> Result<i32, CommandSyntaxError> {
    give_with_duration(context, true)
}

fn give_with_duration(
    context: &SteelCommandContext<CommandSource>,
    infinite: bool,
) -> Result<i32, CommandSyntaxError> {
    let targets = context.entities("targets")?;
    let effect = parse_effect(context)?;
    let instantaneous = MOB_EFFECT_BEHAVIORS
        .get_behavior(effect)
        .as_instantaneous()
        .is_some();
    let seconds = if infinite {
        Some(-1)
    } else {
        context.integer("seconds").ok()
    };
    let duration = match seconds {
        Some(seconds) if instantaneous || seconds == -1 => seconds,
        Some(seconds) => seconds * 20,
        None if instantaneous => 1,
        None => 600,
    };
    let amplifier = context.integer("amplifier").unwrap_or(0);
    let visible = !context.boolean("hideParticles").unwrap_or(false);

    let mut applied = 0;
    for target in targets {
        if let Some(living) = target.as_living_entity()
            && living.add_mob_effect(
                MobEffectInstance::with_duration(effect, duration, amplifier)
                    .with_visible(visible)
                    .with_show_icon(visible),
            )
        {
            applied += 1;
        }
    }
    if applied == 0 {
        return Err(CommandSyntaxError::dynamic(TextComponent::from(
            &translations::COMMANDS_EFFECT_GIVE_FAILED,
        )));
    }
    context.source().send_success(
        &TextComponent::from(format!("Applied {effect:?} to {applied} entit(y/ies)")),
        false,
    );
    Ok(applied)
}

fn clear_self(context: &SteelCommandContext<CommandSource>) -> Result<i32, CommandSyntaxError> {
    let entity = context.source().entity().ok_or_else(|| {
        CommandSyntaxError::dynamic(TextComponent::from(
            &translations::PERMISSIONS_REQUIRES_ENTITY,
        ))
    })?;
    clear_targets(context, std::slice::from_ref(entity), None)
}

fn clear(context: &SteelCommandContext<CommandSource>) -> Result<i32, CommandSyntaxError> {
    let targets = context.entities("targets")?;
    let only_effect = if context.identifier("effect").is_ok() {
        Some(parse_effect(context)?)
    } else {
        None
    };
    clear_targets(context, &targets, only_effect)
}

fn clear_targets(
    context: &SteelCommandContext<CommandSource>,
    targets: &[SharedEntity],
    only_effect: Option<MobEffectRef>,
) -> Result<i32, CommandSyntaxError> {
    let mut cleared = 0;
    for target in targets {
        let Some(living) = target.as_living_entity() else {
            continue;
        };
        if let Some(effect) = only_effect {
            if living.remove_mob_effect(effect) {
                cleared += 1;
            }
        } else {
            let effects = living.active_mob_effects();
            if !effects.is_empty() {
                for effect in effects {
                    living.remove_mob_effect(effect.effect());
                }
                cleared += 1;
            }
        }
    }
    if cleared == 0 {
        let message = if only_effect.is_some() {
            &translations::COMMANDS_EFFECT_CLEAR_SPECIFIC_FAILED
        } else {
            &translations::COMMANDS_EFFECT_CLEAR_EVERYTHING_FAILED
        };
        return Err(CommandSyntaxError::dynamic(TextComponent::from(message)));
    }
    context.source().send_success(
        &TextComponent::from(format!("Cleared effects from {cleared} entit(y/ies)")),
        false,
    );
    Ok(cleared)
}
