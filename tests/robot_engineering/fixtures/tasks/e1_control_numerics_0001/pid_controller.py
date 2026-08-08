"""Synthetic PID controller with anti-windup.

BUG: Lacks anti-windup clamping on the integral term. When output saturates,
the integral term continues to accumulate causing large overshoots.
"""

class PIDController:
    def __init__(self, kp: float, ki: float, kd: float, output_min: float = -100.0, output_max: float = 100.0):
        self.kp = kp
        self.ki = ki
        self.kd = kd
        self.output_min = output_min
        self.output_max = output_max
        self.integral = 0.0
        self.prev_error = 0.0

    def update(self, setpoint: float, measurement: float, dt: float) -> float:
        error = setpoint - measurement
        self.integral += error * dt
        # BUG: no anti-windup — integral grows unbounded when saturated
        derivative = (error - self.prev_error) / dt if dt > 0 else 0.0
        output = self.kp * error + self.ki * self.integral + self.kd * derivative
        # Clamp output
        if output > self.output_max:
            output = self.output_max
        elif output < self.output_min:
            output = self.output_min
        self.prev_error = error
        return output

    def reset(self):
        self.integral = 0.0
        self.prev_error = 0.0
