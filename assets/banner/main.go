// Cinematic banner for the agno README, rendered with MyGo's render plugin.
// Two outputs from one composition: a 60 fps video and a small inline GIF.
// Timing is in seconds so the animation keeps its pace at any frame rate.
//
// Regenerate:
//
//	cd assets/banner
//	go mod tidy
//	go run . -mode video   # banner.mp4  (1400x420, 60 fps, 18 s, H.264)
//	go run . -mode gif     # frames/%04d.png at 12 fps, 1200x360
//	ffmpeg -y -framerate 12 -i frames/%04d.png -filter_complex \
//	  "[0:v]fps=7,scale=1200:-1,split[a][b];[a]palettegen=stats_mode=diff[p];[b][p]paletteuse=dither=sierra2_4a" \
//	  -loop 0 banner.gif   # 18 s, 4.7 MB (under the 5 MB README budget)
package main

import (
	"context"
	"flag"
	"log"
	"math"

	"github.com/egoist/mygo/plugins/render"
	"github.com/egoist/mygo/ui"
)

const seconds = 18

// at converts seconds to the frame units Interpolate expects; fi does the
// same for Sequence/Shift, which take whole frames.
func at(s float64, fps int) float64 { return s * float64(fps) }
func fi(s float64, fps int) int     { return int(math.Round(s * float64(fps))) }

var (
	bgTop    = ui.Hex("#070b14")
	bgMid    = ui.Hex("#0b1220")
	bgBot    = ui.Hex("#101a2c")
	panel    = ui.Hex("#111b2e")
	panelHi  = ui.Hex("#162238")
	line     = ui.RGBA(148, 163, 184, 0.16)
	ink      = ui.Hex("#e8edf7")
	muted    = ui.Hex("#93a4bd")
	dim      = ui.Hex("#64748b")
	accent   = ui.Hex("#f59e0b")
	accentHi = ui.Hex("#fbbf24")
	green    = ui.Hex("#34d399")
	red      = ui.Hex("#f87171")
	yellow   = ui.Hex("#fbbf24")
	greenDot = ui.Hex("#4ade80")
	blue     = ui.RGBA(96, 165, 250, 1)
)

type stat struct {
	value string
	label string
}

var stats = []stat{
	{"4.2 MB", "single static binary"},
	{"106 ms", "1,000 posts, cold build"},
	{"3.3–9.3×", "faster than Hugo"},
}

type stage struct {
	name string
	sub  string
}

var stages = []stage{
	{"content/*.md", "frontmatter + body"},
	{"pulldown-cmark", "parse + highlight"},
	{"MiniJinja", "templates + theme"},
	{"dist/", "HTML · RSS · sitemap"},
}

func composition(fps, w, h int) render.Composition {
	return render.Composition{
		Width: w, Height: h, FPS: float64(fps), Frames: fps * seconds, Scale: 1,
		Dark: true, Background: bgTop,
		View: func(c *ui.Context, f render.Frame) {
			ui.Box(c).Fill().Background(bgTop).Draw(func(p *ui.Painter, r ui.Rect) {
				drawStage(p, r, f, fps)
			})

			smallBrand(c, f, float32(f.Interpolate(
				[]float64{at(2.5, fps), at(3.2, fps)}, []float64{0, 1})*
				f.Interpolate([]float64{at(13.3, fps), at(13.8, fps)}, []float64{1, 0})))

			f.Sequence(c, fi(0, fps), fi(3.5, fps), func(f render.Frame) { intro(c, f, fps) })
			f.Sequence(c, fi(3.3, fps), fi(3.9, fps), func(f render.Frame) { statCards(c, f, fps) })
			f.Sequence(c, fi(7.0, fps), fi(3.6, fps), func(f render.Frame) { terminal(c, f, fps) })
			f.Sequence(c, fi(10.4, fps), fi(3.6, fps), func(f render.Frame) { pipeline(c, f, fps) })
			f.Sequence(c, fi(13.8, fps), 0, func(f render.Frame) { outro(c, f, fps) })
		},
	}
}

// drawStage paints the depth behind everything: graded sky, lights, a
// scrolling perspective grid, dust, and two slowly turning wireframe cubes
// at different depths for parallax.
func drawStage(p *ui.Painter, r ui.Rect, f render.Frame, fps int) {
	p.FillGradient(r, ui.LinearGradient{Angle: 180, Stops: []ui.GradientStop{
		{Color: bgTop, At: 0}, {Color: bgMid, At: 0.52}, {Color: bgBot, At: 1},
	}}, 0)

	p.FillRadialGradient(r, ui.RadialGradient{
		Stops: []ui.GradientStop{
			{Color: ui.RGBA(245, 158, 11, 0.16), At: 0},
			{Color: ui.RGBA(245, 158, 11, 0), At: 1},
		},
		Center: ui.Point{X: 0.5, Y: 0.42}, Radius: 520,
	}, 0)
	p.FillRadialGradient(r, ui.RadialGradient{
		Stops: []ui.GradientStop{
			{Color: ui.RGBA(56, 132, 255, 0.12), At: 0},
			{Color: ui.RGBA(56, 132, 255, 0), At: 1},
		},
		Center: ui.Point{X: 0.86, Y: 0.86}, Radius: 460,
	}, 0)

	// A large, distant cube turning slowly behind the grid.
	drawWireCube(p, r.X+r.W*0.5, r.Y+r.H*0.42, 210,
		f.Seconds()*0.18, 0.32, ui.RGBA(125, 160, 215, 0.10), 1)
	// A nearer, smaller cube turning the other way: parallax.
	drawWireCube(p, r.X+r.W*0.12, r.Y+r.H*0.22, 54,
		-f.Seconds()*0.45, 0.5, ui.RGBA(245, 158, 11, 0.16), 1.4)

	horizon := r.H * 0.66
	vx := r.W * 0.5
	for i := -14; i <= 14; i++ {
		x := vx + float32(i)*r.W*0.095
		a := 0.035 + 0.055*(1-float32(math.Abs(float64(i)))/14)
		p.Line(vx, horizon, x, r.H, 1, ui.RGBA(125, 160, 215, a))
	}
	scroll := f.Seconds() / 2.6
	for i := 0; i < 18; i++ {
		t := (float64(i) + scroll) / 18
		y := horizon + (r.H-horizon)*float32(math.Pow(t, 2.3))
		p.Line(0, y, r.W, y, 1, ui.RGBA(125, 160, 215, float32(0.03+0.13*t)))
	}

	// Two dust layers moving at different speeds.
	for layer := 0; layer < 2; layer++ {
		speed, alpha := 8.0, 0.22
		if layer == 1 {
			speed, alpha = 15.0, 0.13
		}
		for i := 0; i < 45; i++ {
			x := float32((i*617+113)%int(r.W)) / r.W * r.W
			y := float32((i*337+71)%int(r.H*2)) / (r.H * 2) * horizon
			y -= float32(math.Mod(f.Seconds()*speed*float64(1+i%4), float64(horizon)))
			if y < 0 {
				y += horizon
			}
			tw := 0.4 + 0.4*math.Sin(f.Seconds()*14+float64(i)+float64(layer))
			p.Fill(ui.Rect{X: x, Y: y, W: 2, H: 2}, ui.RGBA(191, 210, 240, float32(tw)*float32(alpha)), 1)
		}
	}

	p.FillRadialGradient(r, ui.RadialGradient{
		Stops: []ui.GradientStop{
			{Color: ui.RGBA(0, 0, 0, 0), At: 0.35},
			{Color: ui.RGBA(0, 0, 0, 0.58), At: 1},
		},
	}, 0)
}

// drawWireCube projects an orbiting cube's edges into 2D and strokes them.
func drawWireCube(p *ui.Painter, cx, cy, size float32, yaw, pitch float64, c ui.Color, width float32) {
	v := [8][3]float64{
		{-1, -1, -1}, {1, -1, -1}, {1, 1, -1}, {-1, 1, -1},
		{-1, -1, 1}, {1, -1, 1}, {1, 1, 1}, {-1, 1, 1},
	}
	var pt [8]ui.Point
	for i, q := range v {
		y1 := q[0]*math.Cos(yaw) + q[2]*math.Sin(yaw)
		z1 := -q[0]*math.Sin(yaw) + q[2]*math.Cos(yaw)
		y2 := q[1]*math.Cos(pitch) - z1*math.Sin(pitch)
		z2 := q[1]*math.Sin(pitch) + z1*math.Cos(pitch)
		k := 2.6 / (2.6 + z2*0.55)
		pt[i] = ui.Point{X: cx + float32(y1*k)*size, Y: cy + float32(y2*k)*size}
	}
	edges := [12][2]int{
		{0, 1}, {1, 2}, {2, 3}, {3, 0},
		{4, 5}, {5, 6}, {6, 7}, {7, 4},
		{0, 4}, {1, 5}, {2, 6}, {3, 7},
	}
	for _, e := range edges {
		var path ui.Path
		path.MoveTo(pt[e[0]].X, pt[e[0]].Y).LineTo(pt[e[1]].X, pt[e[1]].Y)
		p.StrokePath(&path, width, c)
	}
}

// smallBrand is the fixed corner mark once the big title has landed.
func smallBrand(c *ui.Context, f render.Frame, a float32) {
	if a <= 0 {
		return
	}
	ui.Box(c).Absolute().Top(26).Left(38).Opacity(a).Children(func() {
		ui.Row(c).AlignItems(ui.Center).Gap(10).Children(func() {
			ui.Box(c).Size(9, 9).Rotate(45).Background(accent).Radius(2)
			ui.Text(c, "agno").FontSize(19).Bold().LetterSpacing(-0.5).TextColor(ink)
			ui.Box(c).Width(1).Height(14).Background(line)
			ui.Text(c, "content-first static sites").FontSize(12).TextColor(dim)
		})
	})
}

// intro lands the name in perspective, then lifts it away for the cards.
func intro(c *ui.Context, f render.Frame, fps int) {
	in := f.SpringIn(ui.Bouncy.Spring, fi(1.0, fps))
	out := f.Interpolate([]float64{at(3.0, fps), at(3.5, fps)}, []float64{1, 0})
	kicker := f.Interpolate([]float64{at(0.4, fps), at(1.3, fps)}, []float64{0, 1})
	tag := f.Interpolate([]float64{at(1.2, fps), at(2.3, fps)}, []float64{0, 1})

	ui.Column(c).Fill().AlignItems(ui.Center).Justify(ui.Center).Gap(0).
		Opacity(float32(out)).Children(func() {
		ui.Text(c, "F A S T   ·   C O N T E N T - F I R S T   ·   R U S T").
			FontSize(13).TextColor(accentHi).LetterSpacing(2).
			Opacity(float32(kicker))
		ui.Box(c).Height(22)
		ry := float32(-20 * (1 - min(in, 1)))
		z := float32(-420 * (1 - min(in, 1)))
		s := float32(0.78 + 0.22*min(in, 1.12))
		ui.Text(c, "agno").FontSize(128).Bold().LetterSpacing(-7).TextColor(accent).
			TextShadow(0, 0, 44, ui.RGBA(245, 158, 11, 0.42)).
			RotateY(ry).TranslateZ(z).Perspective(760).Scale(s, s).
			Opacity(float32(min(in*1.7, 1)))
		ui.Box(c).Height(16)
		ui.Box(c).Width(float32(290 * kicker)).Height(2).Radius(1).Background(accent).
			Opacity(float32(kicker))
		ui.Box(c).Height(20)
		ui.Text(c, "Markdown in. Static site out. One 4.2 MB binary.").
			FontSize(23).TextColor(muted).Opacity(float32(tag)).
			Translate(0, float32(18*(1-tag)))
	})
}

// drawIcon paints a crisp vector mark in the icon tile.
func drawIcon(p *ui.Painter, r ui.Rect, i int) {
	cx, cy := r.X+r.W/2, r.Y+r.H/2
	var path ui.Path
	switch i {
	case 0: // package: an isometric box
		path.MoveTo(cx-8, cy-4).LineTo(cx+4, cy-4).LineTo(cx+4, cy+8).LineTo(cx-8, cy+8).Close()
		path.MoveTo(cx-8, cy-4).LineTo(cx-2, cy-10).LineTo(cx+10, cy-10).LineTo(cx+4, cy-4)
		path.MoveTo(cx+4, cy-4).LineTo(cx+10, cy-10).LineTo(cx+10, cy+2).LineTo(cx+4, cy+8)
	case 1: // stopwatch: a dial with a hand and a crown
		path.Circle(cx, cy+1, 9)
		path.MoveTo(cx, cy+1).LineTo(cx+5, cy-4)
		path.MoveTo(cx, cy-8).LineTo(cx, cy-11)
		path.MoveTo(cx-4, cy-11).LineTo(cx+4, cy-11)
	case 2: // growth: rising bars under a trend arrow
		path.MoveTo(cx-10, cy+9).LineTo(cx+10, cy+9)
		path.MoveTo(cx-7, cy+9).LineTo(cx-7, cy+3)
		path.MoveTo(cx-1, cy+9).LineTo(cx-1, cy-2)
		path.MoveTo(cx+5, cy+9).LineTo(cx+5, cy-7)
		path.MoveTo(cx+1, cy-4).LineTo(cx+5, cy-8).LineTo(cx+9, cy-4)
	}
	p.StrokePath(&path, 2, accentHi)
}

// statCards flip in from depth, settle at slight angles, then breathe.
func statCards(c *ui.Context, f render.Frame, fps int) {
	out := f.Interpolate([]float64{at(3.3, fps), at(3.9, fps)}, []float64{1, 0})
	head := f.Interpolate([]float64{at(0, fps), at(0.75, fps)}, []float64{0, 1})

	ui.Column(c).Fill().AlignItems(ui.Center).Justify(ui.Center).Gap(26).
		Opacity(float32(out)).Children(func() {
		ui.Text(c, "BUILT FOR SITES WHERE CONTENT IS THE PRODUCT").
			FontSize(13).TextColor(accentHi).LetterSpacing(2.4).Opacity(float32(head))
		ui.Row(c).Gap(20).Children(func() {
			for i, s := range stats {
				g, _ := f.Shift(fi(0.25+0.5*float64(i), fps), 0)
				sp := g.SpringIn(ui.Snappy.Spring, fi(0.7, fps))
				e := min(sp, 1)
				floatY := float32(4 * math.Sin(f.Seconds()*0.7+float64(i)*1.7) * e)
				z := float32(-500*(1-e)) + float32(30*e)
				ry := float32(95*(1-e)) + float32(3-2*i)*float32(e)
				rx := float32(18*(1-e)) + float32(4-3*i)*float32(e)
				ui.Column(c).Width(358).Padding(24, 26).Gap(12).Radius(20).
					Background(panel).Border(1, ui.RGBA(245, 158, 11, 0.20)).
					Shadow(0, 26, 54, -8, ui.RGBA(0, 0, 0, 0.62)).
					RotateY(ry).RotateX(rx).TranslateZ(z).Perspective(900).
					Translate(0, float32(46*(1-e))+floatY).
					Opacity(float32(min(sp*1.8, 1))).Children(func() {
					ui.Row(c).AlignItems(ui.Center).Gap(13).Children(func() {
						ui.Box(c).Size(44, 44).Radius(12).
							Background(ui.RGBA(245, 158, 11, 0.12)).
							Border(1, ui.RGBA(245, 158, 11, 0.22)).
							Draw(func(p *ui.Painter, r ui.Rect) { drawIcon(p, r, i) })
						ui.Column(c).Gap(2).Children(func() {
							ui.Text(c, s.value).FontSize(42).Bold().LetterSpacing(-1.6).TextColor(ink)
							ui.Text(c, s.label).FontSize(15).TextColor(muted)
						})
					})
					ui.Box(c).Height(1).Background(line)
					ui.Text(c, "AGNO 0.3.7").FontSize(10).TextColor(dim).LetterSpacing(1.4)
				})
			}
		})
	})
}

// terminal tilts into the frame like a real monitor lit from the side.
func terminal(c *ui.Context, f render.Frame, fps int) {
	in := f.SpringIn(ui.Snappy.Spring, fi(0.8, fps))
	out := f.Interpolate([]float64{at(3.0, fps), at(3.6, fps)}, []float64{1, 0})
	e := min(in, 1)
	cmd := "$ agno build"
	typed := cmd[:int(f.Interpolate([]float64{at(0.4, fps), at(1.4, fps)}, []float64{0, float64(len(cmd))}))]
	cursor := f.Seconds() < 1.5 && int(f.Seconds()*8)%2 == 0
	line1 := f.Interpolate([]float64{at(1.5, fps), at(1.83, fps)}, []float64{0, 1})
	line2 := f.Interpolate([]float64{at(1.83, fps), at(2.17, fps)}, []float64{0, 1})
	line3 := f.Interpolate([]float64{at(2.17, fps), at(2.5, fps)}, []float64{0, 1})
	bar := f.Interpolate([]float64{at(1.7, fps), at(2.8, fps)}, []float64{0, 1})

	ui.Column(c).Fill().AlignItems(ui.Center).Justify(ui.Center).Gap(20).
		Opacity(float32(out)).Children(func() {
		ui.Text(c, "ONE COMMAND. A WHOLE SITE.").FontSize(13).TextColor(accentHi).
			LetterSpacing(2.4).Opacity(float32(f.Interpolate([]float64{at(0, fps), at(0.67, fps)}, []float64{0, 1})))
		ui.Column(c).Width(920).Padding(0, 26, 24).Radius(18).
			Background(panelHi).Border(1, ui.RGBA(148, 163, 184, 0.24)).
			Shadow(0, 40, 80, -12, ui.RGBA(0, 0, 0, 0.72)).
			RotateX(float32(13*(1-e) + 3*e)).RotateY(float32(-15*(1-e) - 5*e)).
			TranslateZ(float32(-400 * (1 - e))).Perspective(1050).
			Opacity(float32(min(in*1.5, 1))).Children(func() {
			ui.Row(c).AlignItems(ui.Center).Gap(7).Padding(16, 0).Children(func() {
				ui.Text(c, "●").FontSize(11).TextColor(red)
				ui.Text(c, "●").FontSize(11).TextColor(yellow)
				ui.Text(c, "●").FontSize(11).TextColor(greenDot)
				ui.Box(c).Width(10)
				ui.Text(c, "~/site — agno").Font("monospace").FontSize(13).TextColor(dim)
				ui.Spacer(c)
				ui.Box(c).Padding(4, 10).Radius(6).Background(ui.RGBA(245, 158, 11, 0.12)).Children(func() {
					ui.Text(c, "v0.3.7").Font("monospace").FontSize(10).TextColor(accentHi)
				})
			})
			ui.Box(c).Height(1).Background(line)
			ui.Box(c).Height(18)
			ui.Text(c, typed+map[bool]string{true: "▌", false: ""}[cursor]).
				Font("monospace").FontSize(19).TextColor(ink)
			ui.Box(c).Height(10)
			ui.Text(c, "✓  1,000 pages rendered").Font("monospace").FontSize(15).
				TextColor(green).Opacity(float32(line1))
			ui.Text(c, "✓  RSS · sitemap · search index written").Font("monospace").FontSize(15).
				TextColor(green).Opacity(float32(line2))
			ui.Text(c, "Built 1,000 pages in 106 ms  →  ./dist").Font("monospace").FontSize(16).
				TextColor(accentHi).Opacity(float32(line3))
			ui.Box(c).Height(14)
			ui.Box(c).Width(float32(868 * bar)).Height(3).Radius(2).Background(accent)
		})
	})
}

// pipeline walks the build path as four 3D tiles, with light travelling
// along the connections between them.
func pipeline(c *ui.Context, f render.Frame, fps int) {
	out := f.Interpolate([]float64{at(3.0, fps), at(3.6, fps)}, []float64{1, 0})
	head := f.Interpolate([]float64{at(0, fps), at(0.7, fps)}, []float64{0, 1})
	flow := f.Seconds()

	ui.Column(c).Fill().AlignItems(ui.Center).Justify(ui.Center).Gap(24).
		Opacity(float32(out)).Children(func() {
		ui.Text(c, "MARKDOWN  →  PARSE  →  TEMPLATES  →  STATIC OUTPUT").
			FontSize(13).TextColor(accentHi).LetterSpacing(2.2).Opacity(float32(head))
		ui.Row(c).Gap(10).AlignItems(ui.Center).Children(func() {
			for i, s := range stages {
				g, _ := f.Shift(fi(0.2+0.42*float64(i), fps), 0)
				sp := g.SpringIn(ui.Snappy.Spring, fi(0.7, fps))
				e := min(sp, 1)
				z := float32(-420 * (1 - e))
				ry := float32(-55 * (1 - e))
				rise := float32(30 * (1 - e))
				ui.Column(c).Width(268).Padding(18, 20).Gap(7).Radius(16).
					Background(panel).Border(1, ui.RGBA(96, 165, 250, 0.22)).
					Shadow(0, 22, 48, -10, ui.RGBA(0, 0, 0, 0.6)).
					RotateY(ry).TranslateZ(z).Perspective(850).
					Translate(0, rise).Opacity(float32(min(sp*1.8, 1))).Children(func() {
					ui.Text(c, string(rune('0'+i+1))).FontSize(11).TextColor(blue).LetterSpacing(2)
					ui.Text(c, s.name).Font("monospace").FontSize(19).TextColor(ink)
					ui.Text(c, s.sub).FontSize(13).TextColor(muted)
				})
				if i < len(stages)-1 {
					ui.Box(c).Width(20).Height(2).Radius(1).Draw(func(p *ui.Painter, r ui.Rect) {
						var path ui.Path
						path.MoveTo(r.X, r.Y+r.H/2).LineTo(r.X+r.W, r.Y+r.H/2)
						p.StrokePath(&path, 2, ui.RGBA(96, 165, 250, 0.28))
						t := float32(math.Mod(flow*1.6+float64(i)*0.23, 1))
						p.Fill(ui.Rect{X: r.X + r.W*t - 3, Y: r.Y - 2, W: 6, H: 6}, accentHi, 3)
					})
				}
			}
		})
		ui.Text(c, "Content is the product — no plugin runtime, no node_modules.").
			FontSize(16).TextColor(muted).
			Opacity(float32(f.Interpolate([]float64{at(1.6, fps), at(2.4, fps)}, []float64{0, 1})))
	})
}

// outro holds on the mark long enough to be read, then settles with a slow
// pulse on the docs address.
func outro(c *ui.Context, f render.Frame, fps int) {
	in := f.Interpolate([]float64{at(0, fps), at(0.7, fps)}, []float64{0, 1})
	mark := f.SpringIn(ui.Snappy.Spring, fi(0.9, fps))
	tagline := f.Interpolate([]float64{at(0.6, fps), at(1.4, fps)}, []float64{0, 1})
	docs := f.Interpolate([]float64{at(1.3, fps), at(2.2, fps)}, []float64{0, 1})
	pulse := 0.5 + 0.5*math.Sin(f.Seconds()*0.85)
	s := float32(0.95 + 0.05*min(mark, 1))

	ui.Column(c).Fill().AlignItems(ui.Center).Justify(ui.Center).Gap(16).
		Opacity(float32(in)).Children(func() {
		ui.Box(c).Absolute().Fill().Draw(func(p *ui.Painter, r ui.Rect) {
			drawWireCube(p, r.X+r.W/2, r.Y+r.H/2, 120, f.Seconds()*0.3, 0.4,
				ui.RGBA(245, 158, 11, 0.14), 1)
		})
		ui.Text(c, "agno").FontSize(98).Bold().LetterSpacing(-6).TextColor(accent).
			TextShadow(0, 0, float32(34+24*pulse), ui.RGBA(245, 158, 11, 0.38)).
			Scale(s, s).Translate(0, float32(12*(1-min(mark, 1))))
		ui.Text(c, "A fast, content-first static site generator written in Rust.").
			FontSize(21).TextColor(muted).Opacity(float32(tagline))
		ui.Box(c).Padding(10, 20).Radius(10).Background(ui.RGBA(245, 158, 11, 0.10)).
			Border(1, ui.RGBA(245, 158, 11, 0.32)).
			Opacity(float32(docs)).Children(func() {
			ui.Text(c, "agno-docs.pages.dev").Font("monospace").FontSize(15).TextColor(accentHi)
		})
	})
}

func main() {
	mode := flag.String("mode", "video", "video (banner.mp4, 60 fps) or gif (frames/%04d.png, 12 fps)")
	out := flag.String("o", "", "output path")
	flag.Parse()

	ctx := context.Background()
	if *mode == "gif" {
		path := *out
		if path == "" {
			path = "frames/%04d.png"
		}
		err := render.Images(ctx, composition(12, 1200, 360), path, render.Options{})
		if err != nil {
			log.Fatal(err)
		}
		return
	}

	path := *out
	if path == "" {
		path = "banner.mp4"
	}
	opts := render.Options{Encoder: render.FFmpeg, Codec: render.H264,
		Progress: func(p render.Progress) {
			if p.Done%(6*60) == 0 || p.Done == p.Frames {
				log.Printf("%d/%d frames", p.Done, p.Frames)
			}
		},
	}
	err := render.Video(ctx, composition(60, 1400, 420), path, opts)
	if err != nil {
		log.Fatal(err)
	}
}
