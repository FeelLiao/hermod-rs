import 'package:flutter/physics.dart';
import 'package:material_ui/material_ui.dart';

/// Identity-triggered opacity only: retains the child element and its state.
/// Navigation passes its selected destination; progress must not be an identity.
/// Interrupted fades continue from their current opacity; no ticker repeats.
class MotionSwap extends StatefulWidget {
  const MotionSwap({
    super.key,
    required this.child,
    this.identity,
    this.enabled = true,
  });
  final Widget child;
  final Object? identity;
  final bool enabled;
  @override
  State<MotionSwap> createState() => _MotionSwapState();
}

class _MotionSwapState extends State<MotionSwap>
    with SingleTickerProviderStateMixin {
  late final AnimationController _controller = AnimationController(
    vsync: this,
    value: 1,
  );
  // Effects springs are critically damped, unlike spatial (bouncy) springs.
  // Clamp as a final paint boundary, even if future tokens change damping.
  late final Animation<double> _opacity = _controller.drive(
    Tween<double>(begin: 0, end: 1).chain(CurveTween(curve: _UnitClamp())),
  );
  bool _reduced = false;
  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    _reduced =
        MediaQuery.disableAnimationsOf(context) ||
        MediaQuery.accessibleNavigationOf(context);
    if (_reduced || !widget.enabled) _controller.value = 1;
  }

  @override
  void didUpdateWidget(covariant MotionSwap oldWidget) {
    super.didUpdateWidget(oldWidget);
    if (_reduced || !widget.enabled) {
      _controller.value = 1;
    } else if (widget.identity != oldWidget.identity) {
      final start = _controller.isAnimating ? _controller.value : .35;
      _controller.animateWith(
        SpringSimulation(
          Springs.expressiveFastEffects,
          start,
          1,
          0,
          snapToEnd: true,
        ),
      );
    }
  }

  @override
  void dispose() {
    _controller.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) => FadeTransition(
    opacity: _opacity,
    alwaysIncludeSemantics: true,
    child: widget.child,
  );
}

class _UnitClamp extends Curve {
  @override
  double transformInternal(double t) => t.clamp(0.0, 1.0);
}
