
#![allow(non_snake_case)]

use qmetaobject::*;
use crate::core::gyro_source::{ GyroSource, TimeIMU, TimeQuat };

#[derive(Default, Debug)]
pub struct ChartData {
    pub timestamp_percent: f64,
    pub values: [f64; 4]
}

#[derive(Default)]
struct Series {
    data: Vec<(f64, f64)>, // timestamp, value
    lines: Vec<QLineF>,
    visible: bool,
}

// We can have:
// viewMode 0: Gyro only
// viewMode 0: Gyro + sync results
// viewMode 1: Accel only
// viewMode 2: Quaternions
// viewMode 2: Quaternions + smoothed quaternions

#[derive(Default, QObject)]
pub struct TimelineGyroChart {
    base: qt_base_class!(trait QQuickPaintedItem),

    visibleAreaLeft: qt_property!(f64; WRITE setVisibleAreaLeft),
    visibleAreaRight: qt_property!(f64; WRITE setVisibleAreaRight),
    vscale: qt_property!(f64; WRITE setVScale),

    setAxisVisible: qt_method!(fn(&mut self, a: usize, v: bool)),
    getAxisVisible: qt_method!(fn(&self, a: usize) -> bool),
    axisVisibleChanged: qt_signal!(),

    viewMode: qt_property!(u32; WRITE setViewMode),

    series: [Series; 4+4],

    gyro: Vec<ChartData>,
    accl: Vec<ChartData>,
    quats: Vec<ChartData>,
    smoothed_quats: Vec<ChartData>,
    sync_results: Vec<ChartData>,

    gyro_max: f64,
    duration_ms: f64,
}

impl TimelineGyroChart {
    pub fn setDurationMs(&mut self, v: f64) { self.duration_ms = v; self.update(); }
    fn setVisibleAreaLeft (&mut self, v: f64) { self.visibleAreaLeft = v; self.update(); }
    fn setVisibleAreaRight(&mut self, v: f64) { self.visibleAreaRight = v; self.update(); }
    fn setAxisVisible     (&mut self, a: usize, v: bool) { self.series[a].visible = v; self.update(); self.axisVisibleChanged(); }
    fn getAxisVisible     (&self, a: usize) -> bool { self.series[a].visible }
    fn setVScale          (&mut self, v: f64) { self.vscale = v.max(0.1); self.update(); }
    fn setViewMode        (&mut self, v: u32) { self.viewMode = v; self.update_data(); }
    
    pub fn update(&mut self) {
        self.calculate_lines();
        (self as &dyn QQuickItem).update()
    }

    fn calculate_lines(&mut self) {
        let rect = (self as &dyn QQuickItem).bounding_rect();
        let half_height = rect.height / 2.0;

        let map_to_visible_area = |v: f64| -> f64 { (v - self.visibleAreaLeft) / (self.visibleAreaRight - self.visibleAreaLeft) };

        for serie in &mut self.series  {
            if serie.visible && !serie.data.is_empty() {
                let length = serie.data.len();

                // TODO: take into account offset here, otherwise it picks the wrong range

                let from_index = (self.visibleAreaLeft * length as f64).floor() as usize;
                let mut to_index = (self.visibleAreaRight * (length - 1) as f64).ceil() as usize;
                if from_index >= to_index { to_index = from_index + 1; }
    
                let visible_length = to_index - from_index;
                let mut index_step = 1;
                     if visible_length > 200000 { index_step = 32; }
                else if visible_length > 100000 { index_step = 16; }
                else if visible_length > 50000 { index_step = 8; }
                else if visible_length > 20000 { index_step = 4; }
                else if visible_length > 10000 { index_step = 2; }
    
                //let step = rect.width / (visible_length as f64 / index_step as f64);

                serie.lines.clear();
                serie.lines.reserve(length);
                if serie.data.len() > from_index {
                    let data = serie.data[from_index];
                    let mut prevPoint = QPointF {
                        x: map_to_visible_area(data.0) * rect.width, 
                        y: (1.0 + data.1 * self.vscale) * half_height
                    };
                    let mut i = from_index + 1;
                    while i <= to_index {
                        if serie.data.len() > i {
                            let data = serie.data[i];
                            let point = QPointF { 
                                x: map_to_visible_area(data.0) * rect.width, 
                                y: (1.0 + data.1 * self.vscale) * half_height
                            };
                            
                            serie.lines.push(QLineF { pt1: prevPoint, pt2: point });
                            prevPoint = point;
                        }
                        i += index_step;
                    }
                }
            } else {
                serie.lines.clear();
            }
        }
    }

    fn drawAxis(&mut self, p: &mut QPainter, a: usize, color: &str) {
        let mut pen = QPen::from_color(QColor::from_name(color));
        pen.set_width_f(1.5); // TODO * dpiScale
        p.set_pen(pen);
        
        p.draw_lines(self.series[a].lines.as_slice());
    }

    pub fn setSyncResults(&mut self, data: &[TimeIMU]) {
        self.sync_results = Vec::with_capacity(data.len());

        for x in data {
            self.sync_results.push(ChartData {
                timestamp_percent: x.timestamp / self.duration_ms,
                values: [x.gyro[0], x.gyro[1], x.gyro[2], 0.0]
            });
        }
        Self::normalize_height(&mut self.sync_results, Some(self.gyro_max / (180.0 / std::f64::consts::PI)));

        self.update_data();
    }

    pub fn setFromGyroSource(&mut self, gyro: &GyroSource) {
        self.gyro = Vec::with_capacity(gyro.raw_imu.len());
        self.accl = Vec::with_capacity(gyro.raw_imu.len());
        self.quats = Vec::with_capacity(gyro.quaternions.len());
        self.smoothed_quats = Vec::with_capacity(gyro.smoothed_quaternions.len());

        for x in &gyro.raw_imu {
            self.gyro.push(ChartData {
                timestamp_percent: (x.timestamp + gyro.offset_at_timestamp(x.timestamp)) / self.duration_ms,
                values: [x.gyro[0], x.gyro[1], x.gyro[2], 0.0]
            });
            self.accl.push(ChartData {
                timestamp_percent: (x.timestamp + gyro.offset_at_timestamp(x.timestamp)) / self.duration_ms,
                values: [x.accl[0], x.accl[1], x.accl[2], 0.0]
            });
        }

        let add_quats = |quats: &TimeQuat, out_quats: &mut Vec<ChartData>| {
            for x in quats {
                let mut ts = *x.0 as f64 / 1000.0;
                ts += gyro.offset_at_timestamp(ts);
    
                let q = x.1.quaternion().as_vector();

                out_quats.push(ChartData {
                    timestamp_percent: ts / self.duration_ms,
                    values: [q[0], q[1], q[2], q[3]]
                });
            }
        };
        add_quats(&gyro.quaternions, &mut self.quats);
        add_quats(&gyro.org_smoothed_quaternions, &mut self.smoothed_quats);

        self.gyro_max = Self::normalize_height(&mut self.gyro, None);
        Self::normalize_height(&mut self.accl, None);
        let qmax = Self::normalize_height(&mut self.quats, None);
        Self::normalize_height(&mut self.smoothed_quats, Some(qmax));

        self.update_data();
    }
    fn get_serie_vector(vec: &Vec<ChartData>, i: usize) -> Vec<(f64, f64)> {
        let mut ret = Vec::with_capacity(vec.len());
        for x in vec {
            ret.push((x.timestamp_percent, x.values[i]));
        }
        ret
    }
    pub fn update_data(&mut self) {
        for s in &mut self.series {
            s.data.clear();
        }
        match self.viewMode {
            0 => {  // Gyroscope
                self.series[0].data = Self::get_serie_vector(&self.gyro, 0);
                self.series[1].data = Self::get_serie_vector(&self.gyro, 1);
                self.series[2].data = Self::get_serie_vector(&self.gyro, 2);

                // + Sync results
                self.series[4].data = Self::get_serie_vector(&self.sync_results, 0);
                self.series[5].data = Self::get_serie_vector(&self.sync_results, 1);
                self.series[6].data = Self::get_serie_vector(&self.sync_results, 2);
            }
            1 => { // Accelerometer
                self.series[0].data = Self::get_serie_vector(&self.accl, 0);
                self.series[1].data = Self::get_serie_vector(&self.accl, 1);
                self.series[2].data = Self::get_serie_vector(&self.accl, 2);
            }
            2 => { // Quaternions
                self.series[0].data = Self::get_serie_vector(&self.quats, 0);
                self.series[1].data = Self::get_serie_vector(&self.quats, 1);
                self.series[2].data = Self::get_serie_vector(&self.quats, 2);
                self.series[3].data = Self::get_serie_vector(&self.quats, 3);

                // + Smoothed quaternions
                self.series[4].data = Self::get_serie_vector(&self.smoothed_quats, 0);
                self.series[5].data = Self::get_serie_vector(&self.smoothed_quats, 1);
                self.series[6].data = Self::get_serie_vector(&self.smoothed_quats, 2);
                self.series[7].data = Self::get_serie_vector(&self.smoothed_quats, 3);
            }
            _ => panic!("Invalid view mode")
        }

        self.update();
    }

    fn normalize_height(data: &mut Vec<ChartData>, max: Option<f64>) -> f64 {
        let max = max.unwrap_or_else(|| {
            let mut max = 0.0;
            for x in data.iter() {
                for i in 0..4 {
                    if x.values[i].abs() > max { max = x.values[i].abs(); }
                }
            }
            max
        });
        
        for x in data.iter_mut() {
            for i in 0..4 {
                x.values[i] /= max;
            }
        }
        max
    }
}

impl QQuickItem for TimelineGyroChart {
    fn class_begin(&mut self) {
        self.duration_ms = 1.0;
        self.visibleAreaLeft = 0.0;
        self.visibleAreaRight = 1.0;
        self.vscale = 1.0;
        self.series[0].visible = true;
        self.series[1].visible = true;
        self.series[2].visible = true;

        self.series[4].visible = true;
        self.series[5].visible = true;
        self.series[6].visible = true;
    }

    fn geometry_changed(&mut self, _new: QRectF, _old: QRectF) {
        self.calculate_lines();
        (self as &dyn QQuickItem).update();
    }
}
impl QQuickPaintedItem for TimelineGyroChart {
    fn paint(&mut self, p: &mut QPainter) {
        p.set_render_hint(QPainterRenderHint::Antialiasing, true);

        if self.series[0].visible { self.drawAxis(p, 0, "#8f4c4c"); } // X
        if self.series[1].visible { self.drawAxis(p, 1, "#4c8f4d"); } // Y
        if self.series[2].visible { self.drawAxis(p, 2, "#4c7c8f"); } // Z
        if self.series[3].visible { self.drawAxis(p, 3, "#8f4c8f"); } // Angle

        if self.series[4].visible { self.drawAxis(p, 4, "#f1e427"); } // Sync X d5ce67
        if self.series[5].visible { self.drawAxis(p, 5, "#f7aa0f"); } // Sync Y c9bd4b
        if self.series[6].visible { self.drawAxis(p, 6, "#d3f511"); } // Sync Z a89c30
        if self.series[7].visible { self.drawAxis(p, 7, "#11f2f5"); } // Sync Angle

        //if self.series[8].visible { self.drawAxis(p, 8, "#67d793"); } // Smoothed X
        //if self.series[9].visible { self.drawAxis(p, 9, "#4aca7d"); } // Smoothed Y
        //if self.series[10].visible { self.drawAxis(p, 10, "#30a860"); } // Smoothed Z
        //if self.series[11].visible { self.drawAxis(p, 11, "#00cc51"); } // Smoothed Angle
    }
}
