//! Evaluación de umbrales. La lógica es pura: recibe métricas y devuelve
//! eventos, sin leer archivos ni escribir en la salida.

use crate::config::Config;

/// Métrica que superó su umbral.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Metric {
    Cpu,
    Memory,
    Disk,
    Temperature,
}

impl Metric {
    /// Etiqueta usada en la salida legible.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Cpu => "CPU",
            Self::Memory => "Memoria",
            Self::Disk => "Disco",
            Self::Temperature => "Temperatura",
        }
    }
}

/// Alerta disparada, con el valor que la provocó.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Alert {
    pub metric: Metric,
    /// Puntos base para CPU, memoria y disco; miligrados para temperatura.
    pub value: u64,
}

/// Compara las métricas con los umbrales de la configuración.
pub fn evaluate(
    config: &Config,
    cpu: u32,
    memory: u32,
    disk: u32,
    temperature: Option<i64>,
) -> Vec<Alert> {
    let mut alerts = Vec::new();
    if cpu >= config.cpu_alert {
        alerts.push(Alert {
            metric: Metric::Cpu,
            value: u64::from(cpu),
        });
    }
    if memory >= config.memory_alert {
        alerts.push(Alert {
            metric: Metric::Memory,
            value: u64::from(memory),
        });
    }
    if disk >= config.disk_alert {
        alerts.push(Alert {
            metric: Metric::Disk,
            value: u64::from(disk),
        });
    }
    if let Some(value) = temperature {
        if value >= config.temperature_alert {
            alerts.push(Alert {
                metric: Metric::Temperature,
                value: value as u64,
            });
        }
    }
    alerts
}

#[cfg(test)]
mod tests {
    use super::{evaluate, Metric};
    use crate::config::Config;

    #[test]
    fn sin_alertas_bajo_los_umbrales() {
        let config = Config {
            cpu_alert: 9000,
            memory_alert: 9000,
            disk_alert: 9000,
            temperature_alert: 80_000,
            ..Config::default()
        };
        assert!(evaluate(&config, 1000, 2000, 3000, Some(40_000)).is_empty());
    }

    #[test]
    fn detecta_todas_las_metricas_en_el_limite() {
        let config = Config {
            cpu_alert: 5000,
            memory_alert: 5000,
            disk_alert: 5000,
            temperature_alert: 50_000,
            ..Config::default()
        };
        let alerts = evaluate(&config, 5000, 7500, 9999, Some(50_000));
        assert_eq!(
            alerts.iter().map(|alert| alert.metric).collect::<Vec<_>>(),
            vec![
                Metric::Cpu,
                Metric::Memory,
                Metric::Disk,
                Metric::Temperature
            ]
        );
        assert_eq!(alerts[3].value, 50_000);
    }

    #[test]
    fn temperatura_ausente_nunca_alerta() {
        let config = Config {
            temperature_alert: 0,
            ..Config::default()
        };
        assert!(evaluate(&config, 0, 0, 0, None).is_empty());
    }

    #[test]
    fn etiquetas_son_estables() {
        assert_eq!(Metric::Cpu.label(), "CPU");
        assert_eq!(Metric::Memory.label(), "Memoria");
        assert_eq!(Metric::Disk.label(), "Disco");
        assert_eq!(Metric::Temperature.label(), "Temperatura");
    }
}
