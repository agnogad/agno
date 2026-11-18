// Cinematic banner for the agno README, rendered with MyGo's render plugin.
// Timing is in seconds so the animation keeps its pace at any frame rate.
//
// Regenerate:
//
//	cd assets/banner
//	go mod tidy
//	go run .                       # writes frames/%04d.png
//	ffmpeg -y -framerate 30 -i frames/%04d.png -filter_complex \
//	  "[0:v]split[a][b];[a]palettegen=stats_mode=diff[p];[b][p]paletteuse=dither=sierra2_4a" \
//	  -loop 0 banner.gif
package main

import (
	"context"
	"flag"
	"log"
	"math"

	"github.com/egoist/mygo/plugins/render"
	"github.com/egoist/mygo/ui"
)

const (
	fps     = 30
	seconds = 12
	frames  = fps * seconds
	width   = 1400
	height  = 420
)

// at converts seconds to the frame units Interpolate expects; fi does the
// same for Sequence/Shift, which take whole frames.
func at(s float64) float64 { return s * fps }
func fi(s float64) int     { return int(math.Round(s * fps)) }

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

func composition() render.Composition {
	return render.Composition{
		Width: width, Height: height, FPS: fps, Frames: frames, Scale: 1,
		Dark: true, Background: bgTop,
		View: func(c *ui.Context, f render.Frame) {
			ui.Box(c).Fill().Background(bgTop).Draw(func(p *ui.Painter, r ui.Rect) {
				drawStage(p, r, f)
			})

			smallBrand(c, f, float32(f.Interpolate(
				[]float64{at(2.0), at(2.7)}, []float64{0, 1})*
				f.Interpolate([]float64{at(8.6), at(9.1)}, []float64{1, 0})))

			f.Sequence(c, fi(0), fi(3.0), func(f render.Frame) { intro(c, f) })
			f.Sequence(c, fi(2.7), fi(3.5), func(f render.Frame) { statCards(c, f) })
			f.Sequence(c, fi(5.8), fi(3.2), func(f render.Frame) { terminal(c, f) })
			f.Sequence(c, fi(8.7), 0, func(f render.Frame) { outro(c, f) })
		},
	}
}

// drawStage paints the depth behind everything: graded sky, warm and cool
// lights, a scrolling perspective grid and a little dust in the air.
func drawStage(p *ui.Painter, r ui.Rect, f render.Frame) {
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

	horizon := r.H * 0.66
	vx := r.W * 0.5
	for i := -14; i <= 14; i++ {
		x := vx + float32(i)*r.W*0.095
		a := 0.035 + 0.055*(1-float32(math.Abs(float64(i)))/14)
		p.Line(vx, horizon, x, r.H, 1, ui.RGBA(125, 160, 215, a))
	}
	scroll := f.Seconds() / 2.2
	for i := 0; i < 18; i++ {
		t := (float64(i) + scroll) / 18
		y := horizon + (r.H-horizon)*float32(math.Pow(t, 2.3))
		p.Line(0, y, r.W, y, 1, ui.RGBA(125, 160, 215, float32(0.03+0.13*t)))
	}

	for i := 0; i < 90; i++ {
		x := float32((i*617+113)%int(width)) / width * r.W
		y := float32((i*337+71)%int(height*3)) / (height * 3) * horizon
		y -= float32(math.Mod(f.Seconds()*12*float64(2+i%5), float64(horizon)))
		if y < 0 {
			y += horizon
		}
		tw := 0.35 + 0.35*math.Sin(f.Seconds()*19.2+float64(i))
		p.Fill(ui.Rect{X: x, Y: y, W: 2, H: 2}, ui.RGBA(191, 210, 240, float32(tw)*0.28), 1)
	}

	p.FillRadialGradient(r, ui.RadialGradient{
		Stops: []ui.GradientStop{
			{Color: ui.RGBA(0, 0, 0, 0), At: 0.35},
			{Color: ui.RGBA(0, 0, 0, 0.58), At: 1},
		},
	}, 0)
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
func intro(c *ui.Context, f render.Frame) {
	in := f.SpringIn(ui.Bouncy.Spring, fi(1.0))
	out := f.Interpolate([]float64{at(2.5), at(3.0)}, []float64{1, 0})
	kicker := f.Interpolate([]float64{at(0.33), at(1.17)}, []float64{0, 1})
	tag := f.Interpolate([]float64{at(1.0), at(2.0)}, []float64{0, 1})

	ui.Column(c).Fill().AlignItems(ui.Center).Justify(ui.Center).Gap(0).
		Opacity(float32(out)).Children(func() {
		ui.Text(c, "F A S T   ·   C O N T E N T - F I R S T   ·   R U S T").
			FontSize(13).TextColor(accentHi).LetterSpacing(2).
			Opacity(float32(kicker))
		ui.Box(c).Height(22)
		ry := float32(-16 * (1 - min(in, 1)))
		z := float32(-360 * (1 - min(in, 1)))
		s := float32(0.82 + 0.18*min(in, 1.12))
		ui.Text(c, "agno").FontSize(124).Bold().LetterSpacing(-7).TextColor(accent).
			TextShadow(0, 0, 40, ui.RGBA(245, 158, 11, 0.42)).
			RotateY(ry).TranslateZ(z).Perspective(760).Scale(s, s).
			Opacity(float32(min(in*1.7, 1)))
		ui.Box(c).Height(16)
		ui.Box(c).Width(float32(270 * kicker)).Height(2).Radius(1).Background(accent).
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

// statCards fly in from depth, settle at slight angles, then breathe.
func statCards(c *ui.Context, f render.Frame) {
	out := f.Interpolate([]float64{at(2.9), at(3.5)}, []float64{1, 0})
	head := f.Interpolate([]float64{at(0), at(0.75)}, []float64{0, 1})

	ui.Column(c).Fill().AlignItems(ui.Center).Justify(ui.Center).Gap(26).
		Opacity(float32(out)).Children(func() {
		ui.Text(c, "BUILT FOR SITES WHERE CONTENT IS THE PRODUCT").
			FontSize(13).TextColor(accentHi).LetterSpacing(2.4).Opacity(float32(head))
		ui.Row(c).Gap(20).Children(func() {
			for i, s := range stats {
				g, _ := f.Shift(fi(0.25+0.5*float64(i)), 0)
				sp := g.SpringIn(ui.Snappy.Spring, fi(0.6))
				e := min(sp, 1)
				floatY := float32(4 * math.Sin(f.Seconds()*0.66+float64(i)*1.7) * e)
				z := float32(-460*(1-e)) + float32(26*e)
				ry := float32(3-2*i)*float32(e) + float32(-14*(1-e))
				rx := float32(4-3*i)*float32(e) + float32(16*(1-e))
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
func terminal(c *ui.Context, f render.Frame) {
	in := f.SpringIn(ui.Snappy.Spring, fi(0.8))
	out := f.Interpolate([]float64{at(2.2), at(2.9)}, []float64{1, 0})
	e := min(in, 1)
	cmd := "$ agno build"
	typed := cmd[:int(f.Interpolate([]float64{at(0.42), at(1.42)}, []float64{0, float64(len(cmd))}))]
	cursor := f.Seconds() < 1.5 && int(f.Seconds()*8)%2 == 0
	line1 := f.Interpolate([]float64{at(1.5), at(1.83)}, []float64{0, 1})
	line2 := f.Interpolate([]float64{at(1.83), at(2.17)}, []float64{0, 1})
	line3 := f.Interpolate([]float64{at(2.17), at(2.5)}, []float64{0, 1})
	bar := f.Interpolate([]float64{at(1.67), at(2.67)}, []float64{0, 1})

	ui.Column(c).Fill().AlignItems(ui.Center).Justify(ui.Center).Gap(20).
		Opacity(float32(out)).Children(func() {
		ui.Text(c, "ONE COMMAND. A WHOLE SITE.").FontSize(13).TextColor(accentHi).
			LetterSpacing(2.4).Opacity(float32(f.Interpolate([]float64{at(0), at(0.67)}, []float64{0, 1})))
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

// outro holds on the mark long enough to be read, then settles with a slow
// pulse on the docs address.
func outro(c *ui.Context, f render.Frame) {
	in := f.Interpolate([]float64{at(0), at(0.6)}, []float64{0, 1})
	mark := f.SpringIn(ui.Snappy.Spring, fi(0.8))
	tagline := f.Interpolate([]float64{at(0.5), at(1.2)}, []float64{0, 1})
	docs := f.Interpolate([]float64{at(1.1), at(1.8)}, []float64{0, 1})
	pulse := 0.5 + 0.5*math.Sin(f.Seconds()*0.9)
	s := float32(0.96 + 0.04*min(mark, 1))

	ui.Column(c).Fill().AlignItems(ui.Center).Justify(ui.Center).Gap(14).
		Opacity(float32(in)).Children(func() {
		ui.Text(c, "agno").FontSize(88).Bold().LetterSpacing(-5).TextColor(accent).
			TextShadow(0, 0, float32(30+22*pulse), ui.RGBA(245, 158, 11, 0.38)).
			Scale(s, s).Translate(0, float32(10*(1-min(mark, 1))))
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
	out := flag.String("o", "frames/%04d.png", "frame images, with a verb for the frame number")
	flag.Parse()

	err := render.Images(context.Background(), composition(), *out, render.Options{})
	if err != nil {
		log.Fatal(err)
	}
}
